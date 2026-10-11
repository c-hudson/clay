//! Where the GUI's dialogs and windows open from. Menus, actions, local commands and
//! the server's messages all come here, by the web client's names for them. This also
//! follows the connection for the connection log and for a command held back by
//! Connection Lost.

use gpui::{AppContext, Context, Window};

use super::ClayView;
use super::dialogs::small::{Attempt, CertChanged, ConnLog, ConnectionLog, Import, ImportCreds, ImportInsecure, Password, Reconnect};
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
    /// The actions changed since the windows were last told.
    pub actions_changed: bool,
    /// A `/server` waiting to be asked again: where to, and when.
    pub pending_server: Option<(String, std::time::Instant)>,
    /// What the Worlds window shows, hashed: when it changes, the windows are told.
    worlds_sig: u64,
    /// The settings revision the windows were last told of.
    settings_rev: u64,
    /// Settings' Font tab, previewed on this window (`None`: the saved settings).
    pub preview: Option<crate::websocket::GlobalSettingsMsg>,
    /// Saved: the preview goes once the settings move past this revision.
    preview_until: Option<u64>,
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
            "import" => self.open_import("", window, cx),
            "change-password" => {
                let v = cx.new(|cx| Password::new(main, window, cx));
                dialogs::open(self, Kind::Password, v, window, cx);
            }
            "spell_check" => self.spell_suggest(window, cx),
            "recent_worlds" => {
                let v = cx.new(|cx| super::dialogs::small::RecentWorlds::new(main, cx));
                dialogs::open(self, Kind::RecentWorlds, v, window, cx);
            }
            "emoji_picker" => {
                let v = cx.new(|cx| super::dialogs::emoji::EmojiPicker::new(main, window, cx));
                dialogs::open(self, Kind::Emoji, v, window, cx);
            }
            "actions" => self.open_actions(None, cx),
            "world-selector" => self.open_worlds(super::windows::worlds::Show::Selector, cx),
            "setup" | "web" | "font" => self.open_settings(super::windows::settings::Tab::from_name(name), cx),
            "notes" => self.open_notes(cx),
            "new-window" => self.open_world_window(None, cx),
            "help" => self.open_window(super::windows::Kind::Help, |main, _w, cx| cx.new(|cx| super::windows::help::HelpView::new(main, cx)), |_, _, _| {}, cx),
            "theme-editor" => {
                use super::windows::{Kind as Win, theme_editor::ThemeEditor};
                self.open_window(
                    Win::ThemeEditor,
                    |main, window, cx| {
                        cx.new(|cx| {
                            let mut e = ThemeEditor::new(main, window, cx);
                            e.follow(cx);
                            e
                        })
                    },
                    |_, _, _| {},
                    cx,
                );
            }
            "keybind-editor" => {
                use super::windows::{Kind as Win, keybind_editor::KeybindEditor};
                self.open_window(Win::KeybindEditor, |main, _window, cx| cx.new(|cx| KeybindEditor::new(main, cx)), |_, _, _| {}, cx);
            }
            "action-editor" => {
                use super::windows::{Kind as Win, action_editor::ActionEditor};
                self.open_window(Win::ActionEditor, |main, window, cx| cx.new(|cx| ActionEditor::new(main, window, cx)), |_, _, _| {}, cx);
            }
            other => crate::debug_log(crate::is_debug_enabled(), &format!("native-gui: {other} is not available yet")),
        }
    }

    /// The Actions window, narrowed to a world for `/actions <world>`.
    pub(super) fn open_actions(&mut self, world: Option<String>, cx: &mut Context<Self>) {
        use super::windows::{Kind as Win, actions::ActionsView};
        let again = world.clone();
        self.open_window(
            Win::Actions,
            move |main, window, cx| cx.new(|cx| ActionsView::new(main, world, window, cx)),
            move |view, window, cx| view.update(cx, |v, cx| v.set_world(again, window, cx)),
            cx,
        );
    }

    /// The Worlds window: the selector, or a world's editor.
    pub(super) fn open_worlds(&mut self, show: super::windows::worlds::Show, cx: &mut Context<Self>) {
        use super::windows::{Kind as Win, worlds::WorldsView};
        let again = show.clone();
        self.open_window(
            Win::Worlds,
            move |main, window, cx| cx.new(|cx| WorldsView::new(main, show, window, cx)),
            move |view, window, cx| view.update(cx, |v, cx| v.show(again, window, cx)),
            cx,
        );
    }

    /// `/worlds` and `/world` sent back by the server (app.js `executeLocalCommand`):
    /// none opens the selector, `-e [name]` an editor (creating the world if it's new),
    /// a name (or `-l name`) switches to that world, which the server has connected.
    pub(super) fn worlds_command(&mut self, args: &[&str], cx: &mut Context<Self>) {
        use super::windows::worlds::Show;
        let find = |v: &Self, name: &str| v.session.core.worlds.iter().position(|w| w.name.eq_ignore_ascii_case(name));
        match args {
            [] => self.open_worlds(Show::Selector, cx),
            ["-e"] => {
                let current = self.session.core.current;
                self.open_worlds(Show::Edit(current), cx);
            }
            ["-e", name @ ..] => {
                let name = name.join(" ");
                match find(self, &name) {
                    Some(i) => self.open_worlds(Show::Edit(i), cx),
                    None => self.send_message(WsMessage::CreateWorld { name }, cx),
                }
            }
            ["-l", name @ ..] | name @ [..] => {
                if let Some(i) = find(self, &name.join(" ")) {
                    self.switch_to(i, cx);
                }
            }
        }
    }

    pub(super) fn open_import(&mut self, prefill: &str, window: &mut Window, cx: &mut Context<Self>) {
        let main = cx.entity().downgrade();
        let v = cx.new(|cx| Import::new(main, prefill, window, cx));
        dialogs::open(self, Kind::Import, v, window, cx);
    }

    /// Run `f` on the content of the window of `kind`, if it's open, in that window.
    pub(super) fn in_window<V: 'static>(&mut self, kind: super::windows::Kind, f: impl FnOnce(&mut V, &mut Window, &mut Context<V>) + 'static, cx: &mut Context<Self>) {
        let (Some(frame), Some(handle)) = (self.windows.frame(kind), self.windows.handle_of(kind)) else { return };
        let Some(view) = frame.read(cx).content::<V>() else { return };
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, cx| view.update(cx, |v, cx| f(v, window, cx)));
        });
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

    /// `/reload`: this GUI starts again as it was started (the server's never restarted
    /// from here, app.js). `/server <addr>`: it starts again attached to `addr`.
    pub(super) fn relaunch(&mut self, target: Option<&str>, cx: &mut Context<Self>) {
        // Running Clay itself: /reload is the server's hot reload (it keeps the MUD
        // connections, and starts this window again with it).
        if target.is_none() && crate::native_gui::runs_clay() {
            let world_index = self.session.core.current;
            self.send_message(WsMessage::SendCommand { world_index, command: "/reload".into() }, cx);
            return;
        }
        let args: Vec<String> = std::env::args().skip(1).collect();
        let args = match target {
            Some(t) => relaunch_args(&args, t),
            None => args,
        };
        if let Err(e) = crate::platform::exec_with_args(args) {
            self.client_line(&format!("% Couldn't start again: {e}"), cx);
        }
    }

    /// `/server ...` (app.js sendCommand's remote-GUI branch): attach to another Clay,
    /// asked twice within 15 seconds; `--cancel` forgets the first. The native GUI only
    /// ever attaches, so there's no `--close` (become a server) here.
    pub(super) fn server_command(&mut self, args: &[&str], cx: &mut Context<Self>) {
        const CONFIRM_WITHIN: std::time::Duration = std::time::Duration::from_secs(15);
        match args {
            [] => self.client_line("Usage: /server host:port  (or)  /server host port  (or)  /server --cancel", cx),
            ["--close", ..] => self.client_line("The native GUI only attaches to a running Clay, so /server --close isn't available here.", cx),
            ["--cancel", ..] => {
                let had = self.surf.pending_server.take().is_some();
                self.client_line(if had { "Cancelled." } else { "No pending /server to cancel." }, cx);
            }
            [host, rest @ ..] => {
                let addr = match rest.first() {
                    Some(port) => format!("{host}:{port}"),
                    None => host.to_string(),
                };
                match self.surf.pending_server.take() {
                    Some((a, at)) if a == addr && at.elapsed() < CONFIRM_WITHIN => self.relaunch(Some(&addr), cx),
                    _ => {
                        self.client_line(
                            &format!("This will disconnect from the current server and attach to {addr} instead. Run /server {addr} again within 15s to confirm, or /server --cancel."),
                            cx,
                        );
                        self.surf.pending_server = Some((addr, std::time::Instant::now()));
                    }
                }
            }
        }
    }

    /// Retry now (the connection log's Retry, Connection Lost's Reconnect).
    pub(super) fn reconnect_now(&mut self, cx: &mut Context<Self>) {
        if let Some(leader) = self.link.leader.clone() {
            // The connection is the main window's.
            cx.defer(move |cx| {
                let _ = leader.update(cx, |v, cx| v.reconnect_now(cx));
            });
            return;
        }
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
            // `/window [world]`: the server connected it; a window of its own shows it.
            WsMessage::OpenWindow { world } => self.open_world_window(world.clone(), cx),
            // The theme editor's themes, and Save's answer.
            WsMessage::ThemeEditorState { themes_json, active_theme, separator_style, .. } => {
                let (j, a, sep) = (themes_json.clone(), active_theme.clone(), separator_style.clone());
                self.in_window::<super::windows::theme_editor::ThemeEditor>(super::windows::Kind::ThemeEditor, move |v, window, cx| v.state(&j, &a, &sep, window, cx), cx);
            }
            WsMessage::ThemeFileSaved { success, error } => {
                let (ok, e) = (*success, error.clone());
                self.in_window::<super::windows::theme_editor::ThemeEditor>(super::windows::Kind::ThemeEditor, move |v, _window, cx| v.saved(ok, e.as_deref(), cx), cx);
            }
            // The keybind editor's state, and Save's answer.
            WsMessage::KeybindEditorState { bindings_json, defaults_json, actions_json } => {
                let (b, d, a) = (bindings_json.clone(), defaults_json.clone(), actions_json.clone());
                self.in_window::<super::windows::keybind_editor::KeybindEditor>(super::windows::Kind::KeybindEditor, move |v, _w, cx| v.state(&b, &d, &a, cx), cx);
            }
            // The Action editor's list.
            WsMessage::ActionEditorState { actions_json, .. } => {
                let j = actions_json.clone();
                self.in_window::<super::windows::action_editor::ActionEditor>(super::windows::Kind::ActionEditor, move |v, window, cx| v.state(&j, window, cx), cx);
            }
            WsMessage::KeybindFileSaved { success, error } => {
                let (ok, e) = (*success, error.clone());
                self.in_window::<super::windows::keybind_editor::KeybindEditor>(super::windows::Kind::KeybindEditor, move |v, _w, cx| v.saved(ok, e.as_deref(), cx), cx);
            }
            // A world's notes, for its notes window.
            WsMessage::NoteEditorState { world_index, world_name, notes } => {
                let kind = super::windows::Kind::Notes(*world_index);
                if let (Some(frame), Some(handle)) = (self.windows.frame(kind), self.windows.handle_of(kind)) {
                    if let Some(view) = frame.read(cx).content::<super::windows::notes::NoteView>() {
                        let (name, notes) = (world_name.clone(), notes.clone());
                        cx.defer(move |cx| {
                            let _ = handle.update(cx, |_, window, cx| view.update(cx, |v, cx| v.set_notes(&name, &notes, window, cx)));
                        });
                    }
                }
            }
            // The MUD wants text edited: over the main window, unless one is open already
            // (a second would throw away unsaved edits).
            WsMessage::McpEditOpen { world_index, reference, name, edit_type, content } => {
                if let Some(open) = self.surf.stack.get::<super::windows::notes::NoteView>(&Kind::McpEdit) {
                    open.update(cx, |v, cx| v.note_ignored(cx));
                } else {
                    let purpose = super::windows::notes::Purpose::Mcp { world_index: *world_index, reference: reference.clone(), edit_type: edit_type.clone(), name: name.clone() };
                    let content = content.clone();
                    self.later(
                        move |v, window, cx| {
                            let main = cx.entity().downgrade();
                            let d = cx.new(|cx| super::windows::notes::NoteView::mcp(main, purpose, content, window, cx));
                            dialogs::open(v, Kind::McpEdit, d, window, cx);
                        },
                        cx,
                    );
                }
            }
            // A world this client asked for: its editor (app.js opens it the same way).
            WsMessage::WorldCreated { world_index } => self.open_worlds(super::windows::worlds::Show::Edit(*world_index), cx),
            // A Fetch's answer, for the world editor that asked.
            WsMessage::ChatLookupResult { request_id, result, .. } => {
                if let Some(frame) = self.windows.frame(super::windows::Kind::Worlds) {
                    if let Some(view) = frame.read(cx).content::<super::windows::worlds::WorldsView>() {
                        let (id, r) = (*request_id, result.clone());
                        let handle = self.windows.handle_of(super::windows::Kind::Worlds);
                        cx.defer(move |cx| {
                            if let Some(h) = handle {
                                let _ = h.update(cx, |_, window, cx| view.update(cx, |v, cx| v.chat_result(id, &r, window, cx)));
                            }
                        });
                    }
                }
            }
            // Their text already came as output; the web does nothing more with them.
            WsMessage::BanListResponse { .. } | WsMessage::UnbanResult { .. } => {}
            _ => return false,
        }
        true
    }

    /// Tell the other windows when what the Worlds window shows has changed (names,
    /// states, addresses, which is current) - not on every line of output.
    pub(super) fn watch_worlds(&mut self, cx: &mut Context<Self>) {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.session.core.current.hash(&mut h);
        for w in &self.session.core.worlds {
            (&w.name, w.connected, w.was_connected).hash(&mut h);
            if let Some(s) = &w.settings {
                (&s.hostname, &s.port, &s.user, &s.world_type, &s.auto_reconnect_secs).hash(&mut h);
            }
        }
        let sig = h.finish();
        if sig != self.surf.worlds_sig {
            self.surf.worlds_sig = sig;
            cx.emit(super::windows::MainEvent::Worlds);
        }
    }

    /// Tell the other windows the settings changed; a saved preview gives way to them.
    pub(super) fn watch_settings(&mut self, cx: &mut Context<Self>) {
        let rev = self.session.core.settings_rev;
        if rev != self.surf.settings_rev {
            self.surf.settings_rev = rev;
            if self.surf.preview_until.is_some_and(|r| rev > r) {
                self.surf.preview = None;
                self.surf.preview_until = None;
                self.apply_settings();
                cx.notify();
            }
            cx.emit(super::windows::MainEvent::Settings);
        }
    }

    /// Draw with these settings instead of the saved ones (`None`: back to the saved).
    pub(super) fn preview_settings(&mut self, s: Option<crate::websocket::GlobalSettingsMsg>, cx: &mut Context<Self>) {
        self.surf.preview = s;
        self.surf.preview_until = None;
        self.apply_settings();
        // The other windows take the Transparency from here.
        self.restyled(cx);
        self.repaint_all(cx);
    }

    /// Settings were saved: keep the preview until the server's answer arrives.
    pub(super) fn keep_preview_until_settings(&mut self, _cx: &mut Context<Self>) {
        if self.surf.preview.is_some() {
            self.surf.preview_until = Some(self.session.core.settings_rev);
        }
    }

    /// The current world's notes, in a window of their own (`/note`, the note icon).
    pub(super) fn open_notes(&mut self, cx: &mut Context<Self>) {
        use super::windows::{Kind as Win, notes::NoteView};
        let index = self.session.core.current;
        let Some(name) = self.session.core.worlds.get(index).map(|w| w.name.clone()) else { return };
        self.open_window(Win::Notes(index), move |main, window, cx| cx.new(|cx| NoteView::notes(main, index, &name, window, cx)), |_, _, _| {}, cx);
    }

    /// `/window [world]` (the server's OpenWindow, which has connected the world) and the
    /// menu's New Window: a whole client in a window, kept to that world if there is one.
    pub(super) fn open_world_window(&mut self, world: Option<String>, cx: &mut Context<Self>) {
        let title = world.as_ref().map_or_else(|| "Clay".to_string(), |w| format!("Clay - {w}"));
        self.open_view(super::viewmode::ViewMode { lock: world, grep: None }, title, (800.0, 600.0), cx);
    }

    /// `/window --grep pattern [-w world] [--regexp]`: the matching lines, half height.
    pub(super) fn open_grep_window(&mut self, pattern: String, world: Option<String>, regex: bool, cx: &mut Context<Self>) {
        let title = format!("Clay - grep: {pattern}");
        let grep = super::viewmode::Grep::new(&pattern, regex);
        self.open_view(super::viewmode::ViewMode { lock: world, grep: Some(grep) }, title, (800.0, 300.0), cx);
    }

    /// A `/window <world>` window: once the worlds are here, show that one, and come back
    /// to it if the server ever shows another (indices change across a reconnect, so by name).
    pub(super) fn keep_to_world(&mut self, cx: &mut Context<Self>) {
        let Some(name) = self.mode.lock.clone() else { return };
        let Some(index) = self.session.core.worlds.iter().position(|w| w.name == name) else { return };
        if self.session.core.current != index {
            let fx = self.session.core.switch_world(index, std::time::Instant::now());
            let rest = self.session.apply(fx);
            self.after(rest, cx);
        }
    }

    /// The Settings window on a tab (`/setup`, `/web`, `/font`, the menu, the icon bar).
    pub(super) fn open_settings(&mut self, tab: super::windows::settings::Tab, cx: &mut Context<Self>) {
        use super::windows::{Kind as Win, settings::{SettingsView, Tab}};
        if tab == Tab::Web && self.session.core.is_multiuser() {
            self.client_line("Web settings are disabled in multiuser mode.", cx);
            return;
        }
        let Some(edit) = self.session.core.settings.clone() else {
            self.client_line("% Settings are available once connected.", cx);
            return;
        };
        self.open_window(
            Win::Settings,
            move |main, window, cx| cx.new(|cx| SettingsView::new(main, edit, tab, window, cx)),
            move |view, _window, cx| view.update(cx, |v, cx| v.show(tab, cx)),
            cx,
        );
    }

    /// Follow the connection: the log's rows, when it shows, and a held command.
    pub(super) fn watch_status(&mut self, cx: &mut Context<Self>) {
        if self.is_follower() {
            // The connection's dialogs are the main window's; a command held here still
            // goes once it's back.
            self.resend_held(cx);
            return;
        }
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
            Some(("help", key)) => dialogs::open_help(self, main, key, window, cx),
            _ => match what {
                "reconnect" => self.connection_lost("look".into(), window, cx),
                "cert" => {
                    let d = cx.new(|cx| CertChanged::new(main, 0, "mud.example.org:4000".into(), "aa11bb22cc33dd44ee55ff6677889900aa11bb22cc33dd44ee55ff6677889900".into(), "0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c4b5a69788796a5b4c3d2e1f0".into(), cx));
                    dialogs::open(self, Kind::CertMismatch, d, window, cx);
                }
                "mcp" => {
                    let msg = WsMessage::McpEditOpen { world_index: 0, reference: "#123.desc".into(), name: "My Room".into(), edit_type: "string-list".into(), content: "A small room with a view.\nThe walls are blue.".into() };
                    self.surface_message(&msg, cx);
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

/// The arguments to start again with for `/server <target>`: this run's, less its
/// target and `--ssh` (a new target is reached directly unless asked again), plus the new
/// one (platform::exec_relaunch's rule for the WebView GUI).
pub(super) fn relaunch_args(args: &[String], target: &str) -> Vec<String> {
    let mut out: Vec<String> = args.iter().filter(|a| *a != "--gui" && !a.starts_with("--gui=") && *a != "--ssh").cloned().collect();
    out.push(format!("--gui={target}"));
    out
}

#[cfg(test)]
mod relaunch_tests {
    use super::relaunch_args;

    #[test]
    fn a_new_target_replaces_the_old_and_drops_ssh() {
        let args: Vec<String> = ["--conf=/x/settings.dat", "--gui=old:9000", "--ssh"].iter().map(|s| s.to_string()).collect();
        assert_eq!(relaunch_args(&args, "new.example:9001"), vec!["--conf=/x/settings.dat".to_string(), "--gui=new.example:9001".to_string()]);
    }
}
