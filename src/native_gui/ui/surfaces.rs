//! Where the GUI's dialogs and windows open from. Menus, actions, local commands and
//! the server's messages all come here, by the web client's names for them. This also
//! follows the connection for the connection log and for a command held back by
//! Connection Lost.

use gpui::{AppContext, Context, Window};

use super::ClayView;
use super::dialogs::small::{Attempt, CertChanged, ConnLog, ConnectionLog, Help, Import, ImportCreds, ImportInsecure, Menu, Password, Reconnect, help_text};
use super::dialogs::{self, Host, Kind};
use super::kit::Kit;
use crate::native_gui::core::client::Status;
use crate::websocket::WsMessage;

type Later = Box<dyn FnOnce(&mut ClayView, &mut Window, &mut Context<ClayView>)>;

/// The connection as the log follows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Seen {
    Idle,
    Connecting(u32),
    LoggingIn,
    Online,
    Login,
    Cert,
    Failed(u32),
    Stopped,
}

/// After this many failures in a row the log offers Retry (app.js: 5 in the WebView).
const FAILURES_FOR_RETRY: u32 = 5;

#[derive(Default)]
pub struct Surfaces {
    pub stack: dialogs::Stack,
    /// What opens or closes on the next frame, where there's a window to do it with.
    later: Vec<Later>,
    seen: Option<Seen>,
    pub conn_log: ConnLog,
    /// A command Connection Lost holds, with the name of the world it was for: sent
    /// there once back online (app.js `pendingReconnectCommand`).
    pub pending_command: Option<(String, String)>,
    /// The `initial_states` count when it was held: it goes after the next one.
    pending_after: u64,
    /// What /import last sent, to send again over plaintext if the user agrees.
    pub pending_import: Option<ImportCreds>,
}

impl Host for ClayView {
    fn dialogs(&mut self) -> &mut dialogs::Stack {
        &mut self.surf.stack
    }

    fn restore_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
    }
}

impl ClayView {
    pub(super) fn kit(&self) -> Kit {
        Kit { t: self.chrome.tokens.clone(), fonts: self.fonts.clone(), field_bg: super::output::hsla(self.palette.bg) }
    }

    /// `--theme-selection-bg`.
    pub(super) fn selection_color(&self) -> gpui::Hsla {
        super::output::hsla([self.theme.selection_bg.r, self.theme.selection_bg.g, self.theme.selection_bg.b])
    }

    /// Put `text` in the input at the cursor (over a selection), as typing it would
    /// (the emoji picker).
    pub(super) fn insert_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.editor.insert(text);
        self.input_touched(cx);
    }

    /// `--theme-prompt`, which the web client's help links use.
    pub(super) fn link_color(&self) -> gpui::Hsla {
        super::output::hsla([self.theme.prompt.r, self.theme.prompt.g, self.theme.prompt.b])
    }

    /// A page of the attached server's web client (`/theme-editor` and the like), for what
    /// the native GUI doesn't draw itself yet.
    fn server_page(&self, page: &str) -> String {
        let ws = self.session.url();
        let base = ws.strip_suffix("/ws").unwrap_or(&ws);
        let base = base.replacen("wss://", "https://", 1).replacen("ws://", "http://", 1);
        format!("{base}/{page}")
    }

    /// Run `f` on the next frame, with the window.
    pub(super) fn later(&mut self, f: impl FnOnce(&mut ClayView, &mut Window, &mut Context<ClayView>) + 'static, cx: &mut Context<Self>) {
        self.surf.later.push(Box::new(f));
        cx.notify();
    }

    /// The frame's queued openings and closings.
    pub(super) fn run_later(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for f in std::mem::take(&mut self.surf.later) {
            f(self, window, cx);
        }
    }

    /// Close a dialog from outside it (the keyboard goes back where it belongs).
    fn close_dialog(&mut self, kind: Kind, cx: &mut Context<Self>) {
        if self.surf.stack.is_open(&kind) {
            self.later(
                move |v, window, cx| {
                    v.surf.stack.close(&kind);
                    if !v.surf.stack.any() {
                        window.focus(&v.focus, cx);
                    }
                },
                cx,
            );
        }
    }

    /// A window or dialog by the web client's name for it: the menu's and icon bar's
    /// items, the bindable actions, and local commands.
    pub(super) fn open_surface(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let main = cx.entity().downgrade();
        match name {
            "menu" => {
                let v = cx.new(|cx| Menu::new(main, cx));
                dialogs::open(self, Kind::Menu, v, window, cx);
            }
            "import" => self.open_import("", window, cx),
            "change-password" => {
                let v = cx.new(|cx| Password::new(main, window, cx));
                dialogs::open(self, Kind::Password, v, window, cx);
            }
            "spell_check" => self.spell_suggest(window, cx),
            "emoji_picker" => {
                let v = cx.new(|cx| super::dialogs::emoji::EmojiPicker::new(main, window, cx));
                dialogs::open(self, Kind::Emoji, v, window, cx);
            }
            "help" => self.open_window(super::windows::Kind::Help, |main, _w, cx| cx.new(|cx| super::windows::help::HelpView::new(main, cx)), |_, _, _| {}, cx),
            // Until the native editors (phase 4h-4j): the server's own pages, in the browser.
            "theme-editor" | "keybind-editor" | "action-editor" => cx.open_url(&self.server_page(name)),
            other => crate::debug_log(crate::is_debug_enabled(), &format!("native-gui: {other} is not available yet")),
        }
    }

    pub(super) fn open_import(&mut self, prefill: &str, window: &mut Window, cx: &mut Context<Self>) {
        let main = cx.entity().downgrade();
        let v = cx.new(|cx| Import::new(main, prefill, window, cx));
        dialogs::open(self, Kind::Import, v, window, cx);
    }

    /// A dialog's "?" (app.js `openPopupHelp`).
    pub(super) fn open_help(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(lines) = help_text(key) else { return };
        let main = cx.entity().downgrade();
        let v = cx.new(|cx| Help::new(main, lines, cx));
        dialogs::open(self, Kind::Help, v, window, cx);
    }

    /// Send a message (dialogs use this).
    pub(super) fn send_message(&mut self, msg: WsMessage, cx: &mut Context<Self>) {
        let fx = self.session.send(msg);
        self.after(fx, cx);
    }

    /// A line of our own in the current world (app.js `appendClientLine`).
    pub(super) fn client_line(&mut self, text: &str, cx: &mut Context<Self>) {
        let fx = self.session.core.client_line(text);
        let rest = self.session.apply(fx);
        self.after(rest, cx);
    }

    /// Run `command` as if it had been typed (the /menu's choices).
    pub(super) fn run_typed(&mut self, command: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.editor.clear();
        self.editor.insert(command);
        self.send_line(window, cx);
    }

    /// Retry now (the connection log's Retry, Connection Lost's Reconnect).
    pub(super) fn reconnect_now(&mut self, cx: &mut Context<Self>) {
        self.surf.conn_log.attempts.clear();
        self.surf.conn_log.retry = false;
        let fx = self.session.core.reconnect_now();
        let rest = self.session.apply(fx);
        self.after(rest, cx);
    }

    /// A command couldn't go: hold it and ask (app.js `showReconnectModal`).
    pub(super) fn connection_lost(&mut self, command: String, window: &mut Window, cx: &mut Context<Self>) {
        let world = self.session.core.current_world().map(|w| w.name.clone()).unwrap_or_default();
        self.surf.pending_command = Some((command, world));
        self.surf.pending_after = self.session.core.initial_states;
        let main = cx.entity().downgrade();
        let v = cx.new(|cx| Reconnect::new(main, cx));
        dialogs::open(self, Kind::Reconnect, v, window, cx);
    }

    /// The server's messages that open or answer a dialog.
    pub(super) fn surface_message(&mut self, msg: &WsMessage, cx: &mut Context<Self>) -> bool {
        match msg {
            WsMessage::CertMismatch { world_index, host, old_fingerprint, new_fingerprint } => {
                let (w, h, o, n) = (*world_index, host.clone(), old_fingerprint.clone(), new_fingerprint.clone());
                self.later(
                    move |v, window, cx| {
                        let main = cx.entity().downgrade();
                        let d = cx.new(|cx| CertChanged::new(main, w, h, o, n, cx));
                        dialogs::open(v, Kind::CertMismatch, d, window, cx);
                    },
                    cx,
                );
            }
            WsMessage::ImportNeedsInsecureConfirm { addr } => {
                let addr = addr.clone();
                self.later(
                    move |v, window, cx| {
                        let main = cx.entity().downgrade();
                        let d = cx.new(|cx| ImportInsecure::new(main, addr, cx));
                        dialogs::open(v, Kind::ImportInsecure, d, window, cx);
                    },
                    cx,
                );
            }
            WsMessage::ImportResult { summary, .. } => self.client_line(summary, cx),
            WsMessage::PasswordChanged { success, error } => {
                if *success {
                    self.close_dialog(Kind::Password, cx);
                    self.client_line("Password changed successfully.", cx);
                } else if let Some(d) = self.surf.stack.get::<Password>(&Kind::Password) {
                    let e = error.clone().unwrap_or_else(|| "Password change failed".into());
                    d.update(cx, |d, cx| {
                        d.error = Some(e);
                        cx.notify();
                    });
                }
            }
            // /connections: lines of our own, unprefixed (app.js `addRawOutputLines`).
            WsMessage::ConnectionsListResponse { lines } => {
                for line in lines {
                    self.client_line(line, cx);
                }
            }
            // Their text already came as output; the web does nothing more with them.
            WsMessage::BanListResponse { .. } | WsMessage::UnbanResult { .. } => {}
            _ => return false,
        }
        true
    }

    /// Follow the connection: the log's rows, when it shows, and a held command.
    pub(super) fn watch_status(&mut self, cx: &mut Context<Self>) {
        let now = match self.session.core.status() {
            Status::Idle => Seen::Idle,
            Status::Connecting { attempt } => Seen::Connecting(*attempt),
            Status::LoggingIn => Seen::LoggingIn,
            Status::Online => Seen::Online,
            Status::NeedLogin { .. } => Seen::Login,
            Status::CertMismatch(_) => Seen::Cert,
            Status::Reconnecting { attempt, .. } => Seen::Failed(*attempt),
            Status::Stopped(_) => Seen::Stopped,
        };
        if self.surf.seen != Some(now) {
            self.surf.seen = Some(now);
            self.status_changed(now, cx);
        }
        self.resend_held(cx);
    }

    fn status_changed(&mut self, now: Seen, cx: &mut Context<Self>) {
        let log = &mut self.surf.conn_log;
        let resolve = |log: &mut ConnLog, ok: bool, note: Option<String>| {
            if let Some(a) = log.attempts.last_mut().filter(|a| a.ok.is_none()) {
                a.ok = Some(ok);
                a.note = note;
                true
            } else {
                false
            }
        };
        match now {
            Seen::Connecting(_) => {
                log.attempts.push(Attempt { url: self.session.url(), ok: None, note: None });
                // Attached to another machine, the log shows while connecting
                // (webview_gui.rs SHOW_CONNECTION_WINDOW).
                if !self.session.target.is_loopback() {
                    self.show_conn_log(cx);
                }
            }
            Seen::LoggingIn => {
                resolve(log, true, None);
            }
            Seen::Online => {
                *log = ConnLog::default();
                self.close_dialog(Kind::ConnectionLog, cx);
            }
            // The login screen or the certificate screen takes over.
            Seen::Login | Seen::Cert => {
                resolve(log, true, None);
                self.close_dialog(Kind::ConnectionLog, cx);
            }
            Seen::Failed(_) | Seen::Stopped => {
                let reason = match self.session.core.status() {
                    Status::Reconnecting { reason, .. } => Some(format!("({reason})")),
                    Status::Stopped(why) => Some(format!("({why})")),
                    _ => None,
                };
                if resolve(log, false, reason) {
                    log.failures += 1;
                }
                if log.failures >= FAILURES_FOR_RETRY || now == Seen::Stopped {
                    log.retry = true;
                    self.show_conn_log(cx);
                }
            }
            Seen::Idle => {}
        }
        cx.notify();
    }

    fn show_conn_log(&mut self, cx: &mut Context<Self>) {
        if self.surf.stack.is_open(&Kind::ConnectionLog) {
            return;
        }
        self.later(
            |v, window, cx| {
                if v.surf.stack.is_open(&Kind::ConnectionLog) {
                    return;
                }
                let main = cx.entity().downgrade();
                let d = cx.new(|cx| ConnectionLog::new(main, cx));
                // It doesn't take the keyboard: it comes and goes on its own.
                dialogs::open_quietly(v, Kind::ConnectionLog, d, window, cx);
            },
            cx,
        );
    }

    /// Back online with a fresh world list: send the held command to its world, by name
    /// (indices change across a reconnect), or put it back in the input if that world is
    /// gone.
    fn resend_held(&mut self, cx: &mut Context<Self>) {
        if self.surf.pending_command.is_none() || !self.session.core.is_online() || self.session.core.initial_states <= self.surf.pending_after {
            return;
        }
        let Some((command, world)) = self.surf.pending_command.take() else { return };
        self.close_dialog(Kind::Reconnect, cx);
        match self.session.core.worlds.iter().position(|w| w.name == world) {
            Some(index) => {
                if index != self.session.core.current {
                    self.switch_to(index, cx);
                }
                let fx = self.session.send(WsMessage::SendCommand { world_index: index, command });
                self.after(fx, cx);
                let fx = self.session.core.command_sent();
                let rest = self.session.apply(fx);
                self.after(rest, cx);
            }
            None => {
                self.editor.clear();
                self.editor.insert(&command);
                cx.notify();
            }
        }
    }
}

#[cfg(feature = "native-gui-snapshot")]
impl ClayView {
    /// The snapshot harness's `open:` step: a dialog by name, with stand-in data for the
    /// ones only the server or a dropped connection opens.
    pub(super) fn snapshot_open(&mut self, what: &str, window: &mut Window, cx: &mut Context<Self>) {
        let main = cx.entity().downgrade();
        match what.split_once('=') {
            Some(("help", key)) => self.open_help(key, window, cx),
            _ => match what {
                "reconnect" => self.connection_lost("look".into(), window, cx),
                "cert" => {
                    let d = cx.new(|cx| CertChanged::new(main, 0, "mud.example.org:4000".into(), "aa11bb22cc33dd44ee55ff6677889900aa11bb22cc33dd44ee55ff6677889900".into(), "0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c4b5a69788796a5b4c3d2e1f0".into(), cx));
                    dialogs::open(self, Kind::CertMismatch, d, window, cx);
                }
                "insecure" => {
                    let d = cx.new(|cx| ImportInsecure::new(main, "192.168.1.20:9000".into(), cx));
                    dialogs::open(self, Kind::ImportInsecure, d, window, cx);
                }
                "connlog" => {
                    let url = self.session.url();
                    self.surf.conn_log.attempts = vec![
                        Attempt { url: url.clone(), ok: Some(false), note: Some("(can't reach host: Connection refused)".into()) },
                        Attempt { url: url.clone(), ok: Some(false), note: Some("(no answer)".into()) },
                        Attempt { url, ok: None, note: None },
                    ];
                    self.surf.conn_log.retry = true;
                    let d = cx.new(|cx| ConnectionLog::new(main, cx));
                    dialogs::open_quietly(self, Kind::ConnectionLog, d, window, cx);
                }
                other => self.open_surface(other, window, cx),
            },
        }
    }
}
