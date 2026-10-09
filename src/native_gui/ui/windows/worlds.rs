//! The Worlds window (app.js `openWorldSelectorPopup`, `openWorldEditorPopup`):
//! - the selector: filter, a table of worlds with their state, Connected-only toggle;
//! - the world editor in its place: Server / Login / Options sections, the rows each
//!   world type shows, the Slack/Discord Fetch picker;
//! - Delete asks first.
//!
//! Picking a world switches the main window (and connects it). The web's Worlds window
//! relays that over IPC; here it's the same session.

use gpui::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, KeyDownEvent, ParentElement,
    Render, ScrollHandle, ScrollWheelEvent, SharedString, StatefulInteractiveElement, Styled, Subscription, WeakEntity, Window, div, prelude::*, px, relative, svg,
};
use gpui_component::IndexPath;
use gpui_component::input::{InputEvent, InputState};
use gpui_component::select::{Select, SelectEvent, SelectState};

use super::MainEvent;
use crate::native_gui::core::client::World;
use crate::native_gui::ui::ClayView;
use crate::native_gui::ui::assets::icon;
use crate::native_gui::ui::chrome::alpha;
use crate::native_gui::ui::dialogs::{self, DialogEvent, Host};
use crate::native_gui::ui::kit::{self, Btn, Kit, ListDown, ListEnter, ListEscape, ListUp};
use crate::native_gui::ui::output::hsla;
use crate::websocket::{ChatDirectoryMsg, ChatPickItem, ChatSettingsUpdate, WsMessage};

/// The world editor's rows that only a MUD-family world shows (app.js
/// `WORLD_EDIT_MUD_FIELD_IDS`; the console's `update_field_visibility`).
const CYAN: [u8; 3] = [0x08, 0x91, 0xb2];

/// The worlds the selector shows: by name, hostname or user, and only connected ones
/// when the toggle is on (app.js `getFilteredWorldIndices`).
pub fn shown(worlds: &[World], filter: &str, only_connected: bool) -> Vec<usize> {
    let f = filter.to_lowercase();
    (0..worlds.len())
        .filter(|&i| {
            let w = &worlds[i];
            if only_connected && !w.connected {
                return false;
            }
            let s = w.settings.as_ref();
            f.is_empty()
                || w.name.to_lowercase().contains(&f)
                || s.is_some_and(|s| s.hostname.to_lowercase().contains(&f) || s.user.to_lowercase().contains(&f))
        })
        .collect()
}

/// "New World", then "New World 2", 3... (app.js `addNewWorld`).
pub fn new_world_name(worlds: &[World]) -> String {
    let taken = |n: &str| worlds.iter().any(|w| w.name.eq_ignore_ascii_case(n));
    let mut name = "New World".to_string();
    let mut k = 1;
    while taken(&name) {
        k += 1;
        name = format!("New World {k}");
    }
    name
}

/// A choice whose labels and values differ (the type, the encoding).
struct Pick {
    state: Entity<SelectState<Vec<SharedString>>>,
    values: Vec<String>,
}

impl Pick {
    fn new(options: &[(&str, &str)], value: &str, window: &mut Window, cx: &mut Context<WorldsView>) -> Self {
        let labels: Vec<SharedString> = options.iter().map(|(_, l)| SharedString::from(l.to_string())).collect();
        let values: Vec<String> = options.iter().map(|(v, _)| v.to_string()).collect();
        let at = values.iter().position(|v| v.eq_ignore_ascii_case(value)).unwrap_or(0);
        let state = cx.new(|cx| SelectState::new(labels, Some(IndexPath::new(at)), window, cx));
        // The rows shown depend on the type and the keep-alive.
        cx.subscribe(&state, |_, _, _: &SelectEvent<Vec<SharedString>>, cx| cx.notify()).detach();
        Self { state, values }
    }

    fn value(&self, cx: &App) -> String {
        self.state.read(cx).selected_index(cx).and_then(|i| self.values.get(i.row)).cloned().unwrap_or_default()
    }
}

/// A Fetch result's list (servers or channels): pick one to fill its text box.
struct ChatList {
    state: Entity<SelectState<Vec<SharedString>>>,
    values: Vec<String>,
}

#[derive(Default)]
struct ChatPicker {
    /// The latest request (an older answer is dropped).
    seq: u64,
    status: Vec<String>,
    error: bool,
    invite: Option<String>,
    servers: Option<ChatList>,
    channels: Option<ChatList>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Server,
    Login,
    Options,
}

struct Editor {
    index: usize,
    from_selector: bool,
    section: Section,
    fields: std::collections::HashMap<&'static str, Entity<InputState>>,
    world_type: Pick,
    auto_login: Pick,
    keep_alive: Pick,
    encoding: Pick,
    ssl: bool,
    logging: bool,
    msp: bool,
    mcp: bool,
    mccp2: bool,
    chat: ChatPicker,
}

impl Editor {
    fn text(&self, name: &str, cx: &App) -> String {
        self.fields.get(name).map(|f| f.read(cx).value().to_string()).unwrap_or_default()
    }
}

pub struct WorldsView {
    main: WeakEntity<ClayView>,
    filter: Entity<InputState>,
    only_connected: bool,
    selected: Option<usize>,
    editor: Option<Editor>,
    stack: dialogs::Stack,
    rows: ScrollHandle,
    wheel: f32,
    _subs: Vec<Subscription>,
}

/// What the Worlds window is asked to show.
#[derive(Clone, Debug)]
pub enum Show {
    Selector,
    /// The editor for this world.
    Edit(usize),
}

impl WorldsView {
    pub fn new(main: WeakEntity<ClayView>, show: Show, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter worlds..."));
        let mut subs = vec![cx.subscribe_in(&filter, window, |this, _, ev: &InputEvent, _w, cx| {
            if let InputEvent::Change = ev {
                let list = this.list(cx);
                if !this.selected.is_some_and(|s| list.contains(&s)) {
                    this.selected = list.first().copied();
                }
                cx.notify();
            }
        })];
        if let Some(m) = main.upgrade() {
            subs.push(cx.subscribe(&m, |_, _, _: &MainEvent, cx| cx.notify()));
        }
        let current = main.upgrade().map(|m| m.read(cx).session.core.current);
        let mut v = Self { main, filter, only_connected: false, selected: current, editor: None, stack: Default::default(), rows: ScrollHandle::new(), wheel: 0.0, _subs: subs };
        v.show(show, window, cx);
        v
    }

    /// The window asked for again (or first): the selector, or an editor.
    pub fn show(&mut self, show: Show, window: &mut Window, cx: &mut Context<Self>) {
        match show {
            Show::Edit(index) => self.edit(index, false, window, cx),
            Show::Selector => {
                if self.editor.is_none() {
                    self.selected = self.main.upgrade().map(|m| m.read(cx).session.core.current);
                    self.filter.update(cx, |f, cx| f.focus(window, cx));
                }
            }
        }
        cx.notify();
    }

    fn worlds<'a>(&self, cx: &'a App) -> &'a [World] {
        self.main.upgrade().map(|m| m.read(cx).session.core.worlds.as_slice()).unwrap_or(&[])
    }

    fn list(&self, cx: &App) -> Vec<usize> {
        let worlds = self.main.upgrade().map(|m| m.read(cx).session.core.worlds.clone()).unwrap_or_default();
        shown(&worlds, &self.filter.read(cx).value(), self.only_connected)
    }

    /// One row up or down; the arrows wrap, the wheel stops at the ends.
    fn step(&mut self, by: isize, wrap: bool, cx: &mut Context<Self>) {
        let list = self.list(cx);
        if list.is_empty() {
            return;
        }
        let at = self.selected.and_then(|s| list.iter().position(|&i| i == s));
        let next = if wrap {
            kit::list_step(at, list.len(), by)
        } else {
            Some(at.map_or(if by > 0 { 0 } else { list.len() - 1 }, |a| (a as isize + by).clamp(0, list.len() as isize - 1) as usize))
        };
        if let Some(n) = next {
            self.selected = Some(list[n]);
            self.rows.scroll_to_item(n);
        }
        cx.notify();
    }

    /// Switch the main window to the selected world and connect it if it can (a world
    /// without an address goes to the server, which opens its editor), then close.
    fn connect(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.selected else { return };
        let Some(w) = self.worlds(cx).get(index).cloned() else { return };
        let has_address = w.settings.as_ref().is_some_and(|s| !s.hostname.is_empty() && !s.port.is_empty());
        let _ = self.main.update(cx, |v, cx| {
            v.switch_to(index, cx);
            let msg = if has_address { WsMessage::ConnectWorld { world_index: index } } else { WsMessage::SendCommand { world_index: index, command: format!("/worlds {}", w.name) } };
            v.send_message(msg, cx);
        });
        cx.emit(DialogEvent::Close);
    }

    fn add(&mut self, cx: &mut Context<Self>) {
        let name = new_world_name(self.worlds(cx));
        // The window stays: WorldCreated turns it into the new world's editor.
        let _ = self.main.update(cx, |v, cx| v.send_message(WsMessage::CreateWorld { name }, cx));
    }

    fn edit(&mut self, index: usize, from_selector: bool, window: &mut Window, cx: &mut Context<Self>) {
        let multiuser = self.main.upgrade().is_some_and(|m| m.read(cx).session.core.is_multiuser());
        if multiuser {
            let _ = self.main.update(cx, |v, cx| v.client_line("World editing is disabled in multiuser mode.", cx));
            return;
        }
        let Some(w) = self.worlds(cx).get(index).cloned() else { return };
        let s = w.settings.clone().unwrap_or_else(empty_settings);
        let mut fields = std::collections::HashMap::new();
        let mut field = |name: &'static str, value: &str, placeholder: &str, cx: &mut Context<Self>| {
            let (v, p) = (value.to_string(), placeholder.to_string());
            fields.insert(name, cx.new(|cx| InputState::new(window, cx).default_value(v).placeholder(p)));
        };
        let token_hint = |has: bool, value: &str, fallback: &'static str| if has && value.is_empty() { "(set - leave blank to keep it)" } else { fallback };
        field("name", &w.name, "", cx);
        field("hostname", &s.hostname, "", cx);
        field("port", &s.port, "", cx);
        field("user", &s.user, "", cx);
        field("password", &s.password, "", cx);
        field("tf_type", &s.tf_type, "(none)", cx);
        field("prompt_wait_ms", &s.prompt_wait_ms.to_string(), "", cx);
        field("slack_token", &s.slack_token, token_hint(s.has_slack_token, &s.slack_token, "xoxb-..."), cx);
        field("slack_app_token", &s.slack_app_token, "xapp-...", cx);
        field("slack_workspace", &s.slack_workspace, "(filled in by Fetch)", cx);
        field("discord_token", &s.discord_token, token_hint(s.has_discord_token, &s.discord_token, "bot token from the Developer Portal"), cx);
        field("discord_guild", &s.discord_guild, "server name (or press Fetch)", cx);
        field("slack_channel", &s.slack_channel, "#channel or @user", cx);
        field("slack_channels", &s.slack_channels, "blank = all channels, or #general,#dev", cx);
        field("discord_channel", &s.discord_channel, "#channel or @user", cx);
        field("discord_channels", &s.discord_channels, "blank = all channels, or #general,#dev", cx);
        field("discord_dm_user", &s.discord_dm_user, "", cx);
        field("auto_reconnect", if s.auto_reconnect_secs.is_empty() { "0" } else { &s.auto_reconnect_secs }, "0, 30, web, web,30", cx);
        field("keep_alive_cmd", &s.keep_alive_cmd, "", cx);
        field("gmcp", &s.gmcp_packages, "", cx);
        let editor = Editor {
            index,
            from_selector,
            section: Section::Server,
            world_type: Pick::new(&[("mud", "MUD"), ("mud_timed_prompt", "MUD - Timed Prompt"), ("slack", "Slack"), ("discord", "Discord")], &s.world_type, window, cx),
            auto_login: Pick::new(&[("Connect", "Connect"), ("Prompt", "Prompt"), ("MOO_prompt", "MOO_prompt"), ("Lines", "Lines"), ("None", "None")], &s.auto_connect_type, window, cx),
            keep_alive: Pick::new(&[("NOP", "NOP"), ("Custom", "Custom"), ("Generic", "Generic"), ("None", "None")], &s.keep_alive_type, window, cx),
            encoding: Pick::new(&[("utf8", "UTF-8"), ("latin1", "Latin1"), ("fansi", "Fansi")], &s.encoding, window, cx),
            ssl: s.use_ssl,
            logging: s.log_enabled,
            msp: s.msp_enabled,
            mcp: s.mcp_enabled,
            mccp2: s.mccp2_enabled,
            chat: ChatPicker::default(),
            fields,
        };
        // A new type starts the Fetch picker over.
        cx.subscribe(&editor.world_type.state, |this, _, _: &SelectEvent<Vec<SharedString>>, cx| {
            if let Some(ed) = &mut this.editor {
                ed.chat = ChatPicker { seq: ed.chat.seq + 1, ..Default::default() };
            }
            cx.notify();
        })
        .detach();
        if let Some(f) = editor.fields.get("name") {
            f.update(cx, |f, cx| f.focus(window, cx));
        }
        self.editor = Some(editor);
        cx.notify();
    }

    /// Back to the selector if it came from there, else the window closes.
    fn close_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let back = self.editor.take().is_some_and(|e| e.from_selector);
        if back {
            self.filter.update(cx, |f, cx| f.focus(window, cx));
            cx.notify();
        } else {
            cx.emit(DialogEvent::Close);
        }
    }

    fn save(&mut self, connect: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ed) = &self.editor else { return };
        if ed.index >= self.worlds(cx).len() {
            return;
        }
        let t = |n: &str| ed.text(n, cx);
        let world_type = ed.world_type.value(cx);
        let prompt_wait_ms = t("prompt_wait_ms").trim().parse::<f64>().ok().filter(|n| n.is_finite()).map_or(1000, |n| n.max(0.0).trunc() as u64);
        let tf_type = t("tf_type").trim().to_string();
        let msg = WsMessage::UpdateWorldSettings {
            world_index: ed.index,
            name: t("name"),
            hostname: t("hostname"),
            port: t("port"),
            user: t("user"),
            password: t("password"),
            use_ssl: ed.ssl,
            log_enabled: ed.logging,
            encoding: ed.encoding.value(cx),
            auto_login: ed.auto_login.value(cx),
            keep_alive_type: ed.keep_alive.value(cx),
            keep_alive_cmd: t("keep_alive_cmd"),
            gmcp_packages: t("gmcp"),
            auto_reconnect_secs: t("auto_reconnect").trim().to_string(),
            msp_enabled: ed.msp,
            mcp_enabled: ed.mcp,
            mccp2_enabled: ed.mccp2,
            world_type,
            prompt_wait_ms,
            tf_type: Some(tf_type),
            slack_token: t("slack_token"),
            slack_channel: t("slack_channel"),
            slack_workspace: t("slack_workspace"),
            discord_token: t("discord_token"),
            discord_guild: t("discord_guild"),
            discord_channel: t("discord_channel"),
            discord_dm_user: t("discord_dm_user"),
            chat: Some(ChatSettingsUpdate {
                discord_guild: t("discord_guild").trim().to_string(),
                discord_channel: t("discord_channel").trim().to_string(),
                discord_channels: t("discord_channels").trim().to_string(),
                slack_channel: t("slack_channel").trim().to_string(),
                slack_channels: t("slack_channels").trim().to_string(),
                slack_workspace: t("slack_workspace").trim().to_string(),
                slack_app_token: t("slack_app_token").trim().to_string(),
            }),
        };
        let index = ed.index;
        let _ = self.main.update(cx, |v, cx| {
            v.send_message(msg, cx);
            if connect {
                v.send_message(WsMessage::ConnectWorld { world_index: index }, cx);
            }
        });
        self.close_editor(window, cx);
    }

    fn ask_delete(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let worlds = self.worlds(cx);
        // The last world can't go.
        if worlds.len() <= 1 {
            return;
        }
        let Some(name) = worlds.get(index).map(|w| w.name.clone()) else { return };
        let main = self.main.clone();
        let me = cx.entity().downgrade();
        dialogs::confirm(
            self,
            main.clone(),
            format!("Delete world '{name}'?"),
            true,
            move |window, cx| {
                let _ = main.update(cx, |v, cx| v.send_message(WsMessage::DeleteWorld { world_index: index }, cx));
                let _ = me.update(cx, |this, cx| {
                    if this.editor.is_some() {
                        this.close_editor(window, cx);
                    }
                });
            },
            window,
            cx,
        );
    }

    fn fetch(&mut self, cx: &mut Context<Self>) {
        let Some(ed) = &mut self.editor else { return };
        let world_type = ed.world_type.value(cx);
        if world_type != "slack" && world_type != "discord" {
            return;
        }
        ed.chat.seq += 1;
        ed.chat.status = vec!["Fetching...".into()];
        ed.chat.error = false;
        let slack = world_type == "slack";
        let msg = WsMessage::ChatLookup {
            request_id: ed.chat.seq,
            world_index: ed.index,
            token: ed.text(if slack { "slack_token" } else { "discord_token" }, cx).trim().to_string(),
            app_token: if slack { ed.text("slack_app_token", cx).trim().to_string() } else { String::new() },
            server: if slack { String::new() } else { ed.text("discord_guild", cx).trim().to_string() },
            world_type,
        };
        let _ = self.main.update(cx, |v, cx| v.send_message(msg, cx));
        cx.notify();
    }

    /// A Fetch's answer (app.js `applyChatDirectory`).
    pub fn chat_result(&mut self, request_id: u64, r: &ChatDirectoryMsg, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ed) = &mut self.editor else { return };
        if request_id != ed.chat.seq {
            return;
        }
        let discord = ed.world_type.value(cx) == "discord";
        let mut lines = Vec::new();
        if !r.error.is_empty() {
            lines.push(r.error.clone());
        } else if r.ok {
            lines.push(if discord {
                format!("Bot: {} - in {} server(s), {} channel(s) listed.", r.bot_name, r.servers.len(), r.channels.len())
            } else {
                format!("Bot: {} - {} channel(s) listed.", r.bot_name, r.channels.len())
            });
        }
        lines.extend(r.warnings.iter().cloned());
        ed.chat.status = lines;
        ed.chat.error = !r.error.is_empty();
        ed.chat.invite = Some(r.invite_url.clone()).filter(|u| u.starts_with("https://"));
        let list = |items: &[ChatPickItem], current: &str, window: &mut Window, cx: &mut Context<Self>| -> Option<ChatList> {
            if items.is_empty() {
                return None;
            }
            let labels: Vec<SharedString> = items.iter().map(|i| SharedString::from(i.label.clone())).collect();
            let at = items.iter().position(|i| chat_spec_matches(current, i));
            let state = cx.new(|cx| SelectState::new(labels, at.map(IndexPath::new), window, cx));
            Some(ChatList { state, values: items.iter().map(|i| i.value.clone()).collect() })
        };
        if discord {
            if ed.text("discord_guild", cx).trim().is_empty() && !r.selected_server.is_empty() {
                let v = r.selected_server.clone();
                if let Some(f) = ed.fields.get("discord_guild") {
                    f.update(cx, |f, cx| f.set_value(v, window, cx));
                }
            }
            let (guild, channel) = (ed.text("discord_guild", cx), ed.text("discord_channel", cx));
            ed.chat.servers = list(&r.servers, &guild, window, cx);
            ed.chat.channels = list(&r.channels, &channel, window, cx);
        } else {
            if !r.selected_server.is_empty() {
                let v = r.selected_server.clone();
                if let Some(f) = ed.fields.get("slack_workspace") {
                    f.update(cx, |f, cx| f.set_value(v, window, cx));
                }
            }
            let channel = ed.text("slack_channel", cx);
            ed.chat.servers = None;
            ed.chat.channels = list(&r.channels, &channel, window, cx);
        }
        // A pick fills its text box; a new server fetches its channels.
        let subs: Vec<(Entity<SelectState<Vec<SharedString>>>, bool)> =
            ed.chat.servers.iter().map(|l| (l.state.clone(), true)).chain(ed.chat.channels.iter().map(|l| (l.state.clone(), false))).collect();
        for (state, is_server) in subs {
            cx.subscribe_in(&state, window, move |this, state, _: &SelectEvent<Vec<SharedString>>, window, cx| {
                let Some(ed) = &mut this.editor else { return };
                let lists = if is_server { &ed.chat.servers } else { &ed.chat.channels };
                let Some(value) = state.read(cx).selected_index(cx).and_then(|i| lists.as_ref()?.values.get(i.row).cloned()) else { return };
                let field = match (is_server, ed.world_type.value(cx).as_str()) {
                    (true, _) => "discord_guild",
                    (false, "discord") => "discord_channel",
                    _ => "slack_channel",
                };
                if let Some(f) = ed.fields.get(field) {
                    f.update(cx, |f, cx| f.set_value(value, window, cx));
                }
                if is_server {
                    this.fetch(cx);
                }
                cx.notify();
            })
            .detach();
        }
        cx.notify();
    }

    fn help(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let main = self.main.clone();
        dialogs::open_help(self, main, key, window, cx);
    }

    fn selector(&mut self, k: &Kit, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = k.t.clone();
        let worlds = self.worlds(cx).to_vec();
        let current = self.main.upgrade().map(|m| m.read(cx).session.core.current);
        let list = self.list(cx);
        let cell = |w: Option<f32>, text: String| {
            let d = div().px(px(8.0)).py(px(4.0)).overflow_hidden().text_ellipsis().whitespace_nowrap().child(text);
            match w {
                Some(w) => d.w(px(w)).flex_shrink_0(),
                None => d.flex_1().min_w_0(),
            }
        };
        let header = div()
            .flex()
            .border_b_1()
            .border_color(t.border)
            .text_color(t.accent)
            .child(cell(Some(24.0), String::new()))
            .child(cell(None, "World".into()))
            .child(cell(Some(170.0), "Hostname".into()))
            .child(cell(Some(64.0), "Port".into()))
            .child(cell(Some(110.0), "User".into()));
        let rows = list.iter().map(|&i| {
            let w = &worlds[i];
            let s = w.settings.as_ref();
            let selected = self.selected == Some(i);
            let is_current = current == Some(i);
            let reconnecting = !w.connected && crate::native_gui::ui::chrome::reconnecting(w);
            let dot_color = match (w.connected, reconnecting, selected) {
                (true, _, true) => hsla([0x00, 0x66, 0x00]),
                (true, _, false) => t.success,
                (false, true, true) => hsla([0x66, 0x5c, 0x00]),
                (false, true, false) => t.highlight,
                (false, false, true) => hsla([0x44, 0x44, 0x44]),
                (false, false, false) => hsla([0x66, 0x66, 0x66]),
            };
            let name = crate::util::strip_ansi_codes(&if w.name.is_empty() { "(unnamed)".to_string() } else { w.name.clone() }).trim().to_string();
            let name_cell = div()
                .px(px(8.0))
                .py(px(4.0))
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .flex()
                .when(is_current, |d| d.child(div().text_color(if selected { gpui::black() } else { t.highlight }).child("* ")))
                .child(name);
            let hover = t.hover;
            let row = div()
                .id(("world-row", i))
                .flex()
                .border_b_1()
                .border_color(t.border)
                .cursor_pointer()
                .child(div().w(px(24.0)).flex_shrink_0().px(px(8.0)).py(px(4.0)).text_color(dot_color).child(if w.connected { "●" } else { "○" }))
                .child(name_cell)
                .child(cell(Some(170.0), s.map(|s| s.hostname.clone()).unwrap_or_default()))
                .child(cell(Some(64.0), s.map(|s| s.port.clone()).unwrap_or_default()))
                .child(cell(Some(110.0), s.map(|s| s.user.clone()).unwrap_or_default()))
                .on_click(cx.listener(move |this, ev: &gpui::ClickEvent, _w, cx| {
                    this.selected = Some(i);
                    if ev.click_count() >= 2 {
                        this.connect(cx);
                    }
                    cx.notify();
                }));
            if selected {
                row.bg(t.accent).text_color(gpui::black())
            } else if is_current {
                row.bg(t.hover).hover(move |s| s.bg(hover))
            } else {
                row.hover(move |s| s.bg(hover))
            }
        });
        let sel = self.selected.filter(|s| *s < worlds.len());
        div()
            .key_context(kit::LIST_CONTEXT)
            .size_full()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .on_action(cx.listener(|this, _: &ListUp, _w, cx| this.step(-1, true, cx)))
            .on_action(cx.listener(|this, _: &ListDown, _w, cx| this.step(1, true, cx)))
            .on_action(cx.listener(|this, _: &ListEnter, _w, cx| this.connect(cx)))
            .on_action(cx.listener(|_, _: &ListEscape, _w, cx| cx.emit(DialogEvent::Close)))
            .child(k.heading("Worlds"))
            .child(k.input(&self.filter).font_family(k.fonts.primary.clone()))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(header)
                    .child(
                        div()
                            .id("world-rows")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .track_scroll(&self.rows)
                            .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, _w, cx| {
                                // A wheel notch is one row; a trackpad's small deltas add up to one.
                                let dy = -f32::from(ev.delta.pixel_delta(px(16.0)).y);
                                if dy.abs() >= 40.0 {
                                    this.wheel = 0.0;
                                    this.step(if dy > 0.0 { 1 } else { -1 }, false, cx);
                                } else {
                                    if dy.signum() != this.wheel.signum() {
                                        this.wheel = 0.0;
                                    }
                                    this.wheel += dy;
                                    if this.wheel.abs() >= 40.0 {
                                        let by = if this.wheel > 0.0 { 1 } else { -1 };
                                        this.wheel = 0.0;
                                        this.step(by, false, cx);
                                    }
                                }
                                cx.stop_propagation();
                            }))
                            .children(rows),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(div().text_size(px(12.0)).child("Connected"))
                            .child(k.toggle("world-only-connected", self.only_connected, cx.listener(|this, _, _w, cx| {
                                this.only_connected = !this.only_connected;
                                if this.only_connected && !this.selected.is_some_and(|s| this.worlds(cx).get(s).is_some_and(|w| w.connected)) {
                                    this.selected = this.worlds(cx).iter().position(|w| w.connected);
                                }
                                cx.notify();
                            }))),
                    )
                    .child(
                        k.buttons()
                            .mt_0()
                            .child(k.help_button("worlds-help", cx.listener(|this, _, w, cx| this.help("worldSelector", w, cx))))
                            .child(div().w(px(12.0)))
                            .child(k.button("world-add", "Add", Btn::Plain, Some(Box::new(cx.listener(|this, _, _w, cx| this.add(cx))))))
                            .child(k.button(
                                "world-edit",
                                "Edit",
                                Btn::Plain,
                                sel.map(|s| -> kit::Click { Box::new(cx.listener(move |this, _, w, cx| this.edit(s, true, w, cx))) }),
                            ))
                            .child(k.button("world-close", "Close", Btn::Plain, Some(Box::new(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))))))
                            .child(k.button(
                                "world-connect",
                                "Connect",
                                Btn::Primary,
                                sel.map(|_| -> kit::Click { Box::new(cx.listener(|this, _, _w, cx| this.connect(cx))) }),
                            )),
                    ),
            )
            .into_any_element()
    }

    fn editor_view(&mut self, k: &Kit, window: &mut Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = k.t.clone();
        let wide = f32::from(window.viewport_size().width) >= 900.0 && f32::from(window.viewport_size().height) >= 640.0;
        let worlds_len = self.worlds(cx).len();
        let Some(ed) = &self.editor else { return div().into_any_element() };
        let ty = ed.world_type.value(cx);
        let is_mud = ty == "mud" || ty == "mud_timed_prompt";
        let (is_slack, is_discord) = (ty == "slack", ty == "discord");
        let custom_keep_alive = ed.keep_alive.value(cx) == "Custom";
        let login_label = if is_slack || is_discord { "Channels" } else { "Login" };
        let mono = k.fonts.primary.clone();
        let input = |name: &str| ed.fields.get(name).map(|f| k.input(f).font_family(mono.clone()).into_any_element()).unwrap_or_else(|| div().into_any_element());
        let row = |label: &'static str, value: gpui::AnyElement| {
            div()
                .flex()
                .items_center()
                .child(div().w(px(120.0)).flex_shrink_0().pr(px(5.0)).text_right().text_color(t.fg2).child(label))
                .child(div().flex_1().min_w_0().child(value))
        };
        let select = |p: &Pick| Select::new(&p.state).w_full().into_any_element();
        let chat_list = |l: &Option<ChatList>| l.as_ref().map(|l| div().mt(px(4.0)).child(Select::new(&l.state).w_full().placeholder("(choose from list...)")));
        let with_pick = |name: &str, l: &Option<ChatList>| div().flex().flex_col().child(input(name)).children(chat_list(l)).into_any_element();
        let mut server = div().flex().flex_col().gap(px(6.0)).child(row("Name", input("name"))).child(row("Type", select(&ed.world_type)));
        if is_mud {
            server = server.child(row("Hostname", input("hostname"))).child(row("Port", input("port"))).child(row(
                "Use SSL",
                k.toggle("world-ssl", ed.ssl, cx.listener(|this, _, _w, cx| {
                    if let Some(ed) = &mut this.editor {
                        ed.ssl = !ed.ssl;
                    }
                    cx.notify();
                }))
                .into_any_element(),
            ));
        }
        if is_slack {
            server = server.child(row("Bot Token", input("slack_token"))).child(row("App Token", input("slack_app_token"))).child(row("Workspace", input("slack_workspace")));
        }
        if is_discord {
            server = server.child(row("Token", input("discord_token"))).child(row("Server", with_pick("discord_guild", &ed.chat.servers)));
        }
        if is_slack || is_discord {
            let status = ed.chat.status.iter().enumerate().map(|(i, l)| div().text_color(if ed.chat.error && i == 0 { t.error } else { t.fg }).child(l.clone()));
            let invite = ed.chat.invite.clone().map(|url| {
                let u = url.clone();
                div()
                    .flex()
                    .gap(px(4.0))
                    .child("Add the bot to a server:")
                    .child(div().id("chat-invite").text_color(hsla(CYAN)).cursor_pointer().child("invite link").on_click(move |_, _, cx| cx.open_url(&u)))
            });
            server = server.child(row(
                "",
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(div().child(k.button("chat-fetch", "Fetch servers & channels", Btn::Plain, Some(Box::new(cx.listener(|this, _, _w, cx| this.fetch(cx)))))))
                    .child(div().text_size(px(12.0)).opacity(0.85).children(status).children(invite))
                    .into_any_element(),
            ));
        }
        let mut login = div().flex().flex_col().gap(px(6.0));
        if is_mud {
            login = login.child(row("User", input("user"))).child(row("Password", input("password"))).child(row("Auto Login", select(&ed.auto_login))).child(row("TF Type", input("tf_type")));
        }
        if ty == "mud_timed_prompt" {
            login = login.child(row("Prompt Wait (ms)", input("prompt_wait_ms")));
        }
        if is_slack {
            login = login.child(row("Send To", with_pick("slack_channel", &ed.chat.channels))).child(row("Show Only", input("slack_channels")));
        }
        if is_discord {
            login = login.child(row("Send To", with_pick("discord_channel", &ed.chat.channels))).child(row("Show Only", input("discord_channels")));
        }
        login = login.child(row("Reconnect", input("auto_reconnect")));
        let mut options = div().flex().flex_col().gap(px(6.0));
        if is_mud {
            options = options.child(row("Keep Alive", select(&ed.keep_alive)));
            if custom_keep_alive {
                options = options.child(row("Keep Alive Cmd", input("keep_alive_cmd")));
            }
            options = options.child(row("Encoding", select(&ed.encoding))).child(row("GMCP", input("gmcp")));
        }
        let toggle_cell = |label: &'static str, id: &'static str, on: bool, flip: fn(&mut Editor), cx: &mut Context<Self>| {
            div().flex().items_center().child(div().flex_1().text_right().pr(px(5.0)).text_color(t.fg2).child(label)).child(k.toggle(id, on, cx.listener(move |this, _, _w, cx| {
                if let Some(ed) = &mut this.editor {
                    flip(ed);
                }
                cx.notify();
            })))
        };
        let mut toggles = div().grid().grid_cols(2).gap(px(6.0)).pt(px(4.0)).child(toggle_cell("Logging", "world-logging", ed.logging, |e| e.logging = !e.logging, cx));
        if is_mud {
            toggles = toggles
                .child(toggle_cell("MSP Sound", "world-msp", ed.msp, |e| e.msp = !e.msp, cx))
                .child(toggle_cell("MCP Edit", "world-mcp", ed.mcp, |e| e.mcp = !e.mcp, cx))
                .child(toggle_cell("MCCP2", "world-mccp2", ed.mccp2, |e| e.mccp2 = !e.mccp2, cx));
        }
        options = options.child(toggles);
        let section_title = |s: &'static str| div().text_size(px(11.0)).font_weight(FontWeight::SEMIBOLD).text_color(hsla(CYAN)).pt(px(6.0)).pb(px(2.0)).child(s.to_uppercase());
        let body: gpui::AnyElement = if wide {
            div()
                .flex()
                .gap(px(28.0))
                .child(div().flex_1().min_w_0().flex().flex_col().gap(px(6.0)).child(section_title("Server")).child(server).child(section_title(login_label)).child(login))
                .child(div().flex_1().min_w_0().flex().flex_col().gap(px(6.0)).child(section_title("Options")).child(options))
                .into_any_element()
        } else {
            match ed.section {
                Section::Server => server.into_any_element(),
                Section::Login => login.into_any_element(),
                Section::Options => options.into_any_element(),
            }
        };
        let nav = (!wide).then(|| {
            let item = |s: Section, ic: &'static str, label: &'static str, cx: &mut Context<Self>| {
                let active = ed.section == s;
                let (fg, hover_fg) = (if active { hsla(CYAN) } else { t.muted }, t.fg2);
                div()
                    .id(label)
                    .w(px(60.0))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(3.0))
                    .py(px(7.0))
                    .px(px(2.0))
                    .rounded(px(7.0))
                    .text_size(px(10.0))
                    .text_color(fg)
                    .cursor_pointer()
                    .relative()
                    .when(active, |d| d.bg(alpha(CYAN, 0.12)).child(div().absolute().left(px(-6.0)).top(px(14.0)).w(px(3.0)).h(px(16.0)).rounded_r(px(2.0)).bg(hsla(CYAN))))
                    .when(!active, |d| d.hover(move |s| s.bg(alpha([255, 255, 255], 0.05)).text_color(hover_fg)))
                    .child(svg().path(icon(ic)).size(px(18.0)).text_color(fg))
                    .child(label)
                    .on_click(cx.listener(move |this, _, _w, cx| {
                        if let Some(ed) = &mut this.editor {
                            ed.section = s;
                        }
                        cx.notify();
                    }))
            };
            div()
                .w(px(72.0))
                .flex_shrink_0()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(4.0))
                .py(px(10.0))
                .border_r_1()
                .border_color(t.border_subtle)
                .bg(k.field_bg)
                .child(item(Section::Server, "we-server", "Server", cx))
                .child(item(Section::Login, "we-login", login_label, cx))
                .child(item(Section::Options, "we-options", "Options", cx))
        });
        let index = ed.index;
        div()
            .size_full()
            .flex()
            .flex_col()
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                if ev.keystroke.key == "escape" {
                    this.close_editor(window, cx);
                    cx.stop_propagation();
                }
            }))
            .child(
                div().px(px(16.0)).pt(px(12.0)).pb(px(2.0)).child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(div().text_size(px(12.0)).font_weight(FontWeight::SEMIBOLD).child("WORLD EDITOR"))
                        .child(k.header("world-editor-close", "", cx.listener(|this, _, w, cx| this.close_editor(w, cx)))),
                ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .children(nav)
                    .child(div().id("world-editor-body").flex_1().min_w_0().overflow_y_scroll().px(px(16.0)).pt(px(8.0)).pb(px(16.0)).child(body)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(16.0))
                    .py(px(12.0))
                    .border_t_1()
                    .border_color(t.border_subtle)
                    .child(k.help_button("world-editor-help", cx.listener(|this, _, w, cx| this.help("worldEditor", w, cx))))
                    .child(k.button(
                        "world-delete",
                        "Delete",
                        Btn::Danger,
                        (worlds_len > 1).then(|| -> kit::Click { Box::new(cx.listener(move |this, _, w, cx| this.ask_delete(index, w, cx))) }),
                    ))
                    .child(div().flex_1())
                    .child(k.button("world-cancel", "Cancel", Btn::Plain, Some(Box::new(cx.listener(|this, _, w, cx| this.close_editor(w, cx))))))
                    .child(k.button("world-save", "Save", Btn::Plain, Some(Box::new(cx.listener(|this, _, w, cx| this.save(false, w, cx))))))
                    .child(k.button("world-save-connect", "Connect", Btn::Primary, Some(Box::new(cx.listener(|this, _, w, cx| this.save(true, w, cx)))))),
            )
            .into_any_element()
    }
}

/// app.js `chatSpecMatches`: does a text box's value ("#general", "general (123)",
/// "123") name this item?
pub fn chat_spec_matches(text: &str, item: &ChatPickItem) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return false;
    }
    if let Some(open) = t.rfind('(') {
        if let Some(id) = t[open + 1..].trim_end().strip_suffix(')') {
            if !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric()) {
                return id == item.id;
            }
        }
    }
    t == item.id || t.trim_start_matches(['#', '@']).eq_ignore_ascii_case(&item.name)
}

/// A world's settings when the server sent none (a fresh world).
fn empty_settings() -> crate::websocket::WorldSettingsMsg {
    serde_json::from_value(serde_json::json!({
        "hostname": "", "port": "", "user": "", "password": "", "use_ssl": false, "log_enabled": false,
        "encoding": "utf8", "auto_connect_type": "Connect", "keep_alive_type": "NOP", "keep_alive_cmd": "",
        "gmcp_packages": "", "has_password": false, "auto_reconnect_secs": "0", "has_notes": false,
        "msp_enabled": true, "mcp_enabled": true, "mccp2_enabled": true, "world_type": "mud", "prompt_wait_ms": 1000,
        "tf_type": "", "slack_token": "", "slack_channel": "", "slack_workspace": "", "discord_token": "",
        "discord_guild": "", "discord_channel": "", "discord_dm_user": "", "discord_channels": "",
        "slack_app_token": "", "slack_channels": "", "has_discord_token": false, "has_slack_token": false
    }))
    .expect("the settings' own fields")
}

impl Host for WorldsView {
    fn dialogs(&mut self) -> &mut dialogs::Stack {
        &mut self.stack
    }

    fn restore_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.editor.as_ref().and_then(|e| e.fields.get("name").cloned()) {
            Some(f) => f.update(cx, |f, cx| f.focus(window, cx)),
            None => self.filter.update(cx, |f, cx| f.focus(window, cx)),
        }
    }
}

impl EventEmitter<DialogEvent> for WorldsView {}
impl Focusable for WorldsView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.filter.read(cx).focus_handle(cx)
    }
}

impl Render for WorldsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(k) = self.main.upgrade().map(|m| m.read(cx).kit()) else { return div().into_any_element() };
        let editing = self.editor.is_some();
        let content = if editing { self.editor_view(&k, window, cx) } else { self.selector(&k, cx) };
        div()
            .id("worlds-window")
            .size_full()
            .relative()
            .when(!editing, |d| d.p(px(12.0)))
            .font_family(k.fonts.primary.clone())
            .text_size(px(13.0))
            .line_height(relative(1.4))
            .text_color(k.t.fg)
            .child(content)
            .children(self.stack.render())
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, name: &str) -> ChatPickItem {
        ChatPickItem { id: id.into(), name: name.into(), label: format!("{name} ({id})"), value: format!("{name} ({id})"), kind: String::new(), can_send: None }
    }

    #[test]
    fn chat_specs_match_as_the_web_matches_them() {
        let general = item("123", "general");
        assert!(chat_spec_matches("#general", &general));
        assert!(chat_spec_matches("General", &general));
        assert!(chat_spec_matches("general (123)", &general));
        assert!(!chat_spec_matches("general (999)", &general), "the id decides");
        assert!(chat_spec_matches("123", &general));
        assert!(!chat_spec_matches("", &general));
    }
}
