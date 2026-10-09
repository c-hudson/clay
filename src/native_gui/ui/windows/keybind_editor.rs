//! The Keybind editor (src/web/keybind-editor.html), as a window of its own:
//! - the actions by category, with their keys;
//! - an action's bindings, a key captured to add one (chords too), Reset to Default and
//!   Clear All;
//! - Find, which captures a key and shows what it's bound to.
//!
//! Every change goes to the server at once (`UpdateKeybindEditorBindings`, so every
//! window uses it); Save writes keybindings.dat; Cancel puts the saved ones back.
//! Captured names come from the same `key_event_to_token` the console and every client
//! resolve keys with.

use std::collections::{BTreeMap, HashSet};
use std::time::{Duration, Instant};

use gpui::{
    App, Context, EventEmitter, FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, KeyBinding, KeyDownEvent, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Subscription, WeakEntity, Window, div, prelude::*, px,
};

use super::MainEvent;
use crate::keybindings::{escape_key_to_token, key_event_to_token};
use crate::keynames::{self, KeySeq, KeyToken, NamedKey};
use crate::native_gui::ui::ClayView;
use crate::native_gui::ui::dialogs::{self, DialogEvent, Host};
use crate::native_gui::ui::kit::{Btn, Kit};
use crate::websocket::WsMessage;

gpui::actions!(keybind_capture, [CapTab, CapShiftTab, CapCtrlC, CapCmdC]);

const CONTEXT: &str = "KeybindCapture";
/// keybind-editor.html: the categories, in order.
const ORDER: [&str; 8] = ["Cursor", "Editing", "History", "Scrollback", "World", "System", "Numeric prefix", "Clay"];
const CHORD_WINDOW: Duration = Duration::from_millis(500);

/// Keys gpui-component's root keeps for itself (focus order, copy): while capturing they
/// are keys like any other.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", CapTab, Some(CONTEXT)),
        KeyBinding::new("shift-tab", CapShiftTab, Some(CONTEXT)),
        KeyBinding::new("ctrl-c", CapCtrlC, Some(CONTEXT)),
        KeyBinding::new("cmd-c", CapCmdC, Some(CONTEXT)),
    ]);
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct ActionInfo {
    pub id: String,
    pub name: String,
    pub category: String,
}

/// A key being captured: keybind-editor.html's chord buffer (Escape or a bound prefix
/// waits for the next key; ^X always may start a chord; ^G cancels one).
#[derive(Default)]
pub struct Capture {
    pending: Vec<KeyToken>,
    armed_at: Option<Instant>,
}

pub enum Captured {
    Pending,
    /// Not a key name (a plain character, a bare modifier).
    NotAKey,
    Name(String),
}

impl Capture {
    pub fn cancel(&mut self) {
        self.pending.clear();
        self.armed_at = None;
    }

    /// One keystroke against the bindings that exist (`bound`).
    pub fn push(&mut self, code: crossterm::event::KeyCode, mods: crossterm::event::KeyModifiers, bound: &[String], now: Instant) -> Captured {
        if !self.pending.is_empty() && self.armed_at.is_some_and(|at| now.duration_since(at) >= CHORD_WINDOW) {
            let seq = KeySeq(std::mem::take(&mut self.pending));
            self.armed_at = None;
            if seq.0 != [KeyToken::Named(NamedKey::Escape)] {
                return Captured::Name(seq.canonical());
            }
        }
        let mid = !self.pending.is_empty();
        let token = if mid { escape_key_to_token(code, mods) } else { key_event_to_token(code, mods) };
        let Some(token) = token else {
            if mid {
                self.cancel();
            }
            return Captured::NotAKey;
        };
        if mid && token == KeyToken::Ctrl('G') {
            self.cancel();
            return Captured::NotAKey;
        }
        if matches!(self.pending.last(), Some(KeyToken::Named(NamedKey::Escape))) {
            self.pending.pop();
            self.pending.push(KeyToken::Esc(Box::new(token)));
        } else {
            self.pending.push(token);
        }
        let candidate = KeySeq(self.pending.clone());
        let escape = matches!(candidate.0.last(), Some(KeyToken::Named(NamedKey::Escape)));
        let prefix = (candidate.0.len() == 1 && candidate.0[0] == KeyToken::Ctrl('X')) || keynames::is_prefix_of_any(bound.iter().map(String::as_str), &candidate);
        if escape || prefix {
            self.armed_at = Some(now);
            return Captured::Pending;
        }
        self.cancel();
        Captured::Name(candidate.canonical())
    }
}

/// What Find found for a key: (action id, from the defaults only).
fn find_matches(key: &str, bindings: &BTreeMap<String, String>, defaults: &BTreeMap<String, String>) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let current = bindings.get(key);
    if let Some(a) = current {
        out.push((a.clone(), false));
    }
    if let Some(d) = defaults.get(key).filter(|d| Some(*d) != current) {
        out.push((d.clone(), true));
    }
    out
}

#[derive(Default)]
struct Find {
    open: bool,
    key: Option<String>,
    matches: Vec<(String, bool)>,
    at: usize,
}

pub struct KeybindEditor {
    main: WeakEntity<ClayView>,
    focus: FocusHandle,
    bindings: BTreeMap<String, String>,
    defaults: BTreeMap<String, String>,
    actions: Vec<ActionInfo>,
    saved: String,
    loaded: bool,
    selected: Option<String>,
    capturing: bool,
    capture: Capture,
    /// A captured key that's another action's: (key, that action).
    conflict: Option<(String, String)>,
    find: Find,
    collapsed: HashSet<String>,
    status: Option<(String, Instant)>,
    stack: dialogs::Stack,
    _subs: Vec<Subscription>,
}

impl KeybindEditor {
    pub fn new(main: WeakEntity<ClayView>, cx: &mut Context<Self>) -> Self {
        let mut subs = Vec::new();
        if let Some(m) = main.upgrade() {
            // A change from anywhere (this editor's echo included) comes back as the
            // settings' keybindings.
            subs.push(cx.subscribe(&m, |this, m, ev: &MainEvent, cx| {
                if let MainEvent::Settings = ev {
                    let json = m.read(cx).session.core.settings.as_ref().map(|s| s.keybindings_json.clone()).unwrap_or_default();
                    if this.loaded && !json.is_empty() {
                        if let Ok(b) = serde_json::from_str(&json) {
                            this.bindings = b;
                        }
                    }
                }
                cx.notify();
            }));
        }
        let _ = main.update(cx, |v, cx| v.send_message(WsMessage::RequestKeybindEditorState, cx));
        Self {
            main,
            focus: cx.focus_handle(),
            bindings: BTreeMap::new(),
            defaults: BTreeMap::new(),
            actions: Vec::new(),
            saved: String::new(),
            loaded: false,
            selected: None,
            capturing: false,
            capture: Capture::default(),
            conflict: None,
            find: Find::default(),
            collapsed: HashSet::new(),
            status: None,
            stack: Default::default(),
            _subs: subs,
        }
    }

    /// The server's state (`KeybindEditorState`).
    pub fn state(&mut self, bindings: &str, defaults: &str, actions: &str, cx: &mut Context<Self>) {
        self.bindings = serde_json::from_str(bindings).unwrap_or_default();
        self.defaults = serde_json::from_str(defaults).unwrap_or_default();
        self.actions = serde_json::from_str(actions).unwrap_or_default();
        self.saved = self.snapshot();
        self.loaded = true;
        cx.notify();
    }

    /// Save's answer (`KeybindFileSaved`).
    pub fn saved(&mut self, success: bool, error: Option<&str>, cx: &mut Context<Self>) {
        if success {
            self.saved = self.snapshot();
            self.say("Saved", cx);
        } else {
            self.say(&format!("Error: {}", error.unwrap_or("unknown")), cx);
        }
    }

    fn snapshot(&self) -> String {
        serde_json::to_string(&self.bindings).unwrap_or_default()
    }

    fn say(&mut self, text: &str, cx: &mut Context<Self>) {
        self.status = Some((text.to_string(), Instant::now()));
        cx.notify();
        cx.spawn(async |this, cx| {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            let _ = this.update(cx, |_, cx| cx.notify());
        })
        .detach();
    }

    fn send(&self, msg: WsMessage, cx: &mut App) {
        let _ = self.main.update(cx, |v, cx| v.send_message(msg, cx));
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        let bindings_json = self.snapshot();
        self.send(WsMessage::UpdateKeybindEditorBindings { bindings_json }, cx);
        cx.notify();
    }

    fn keys_of<'a>(map: &'a BTreeMap<String, String>, action: &str) -> Vec<&'a str> {
        map.iter().filter(|(_, a)| *a == action).map(|(k, _)| k.as_str()).collect()
    }

    fn modified(&self, key: &str) -> bool {
        self.defaults.get(key) != self.bindings.get(key)
    }

    fn name_of(&self, id: &str) -> String {
        self.actions.iter().find(|a| a.id == id).map_or_else(|| id.to_string(), |a| a.name.clone())
    }

    /// A captured key for the selected action: added, unless it's another action's.
    fn apply(&mut self, key: String, cx: &mut Context<Self>) {
        let Some(action) = self.selected.clone() else { return };
        match self.bindings.get(&key) {
            Some(other) if *other != action => self.conflict = Some((key, other.clone())),
            _ => {
                self.bindings.insert(key, action);
                self.capturing = false;
                self.conflict = None;
                self.changed(cx);
            }
        }
        cx.notify();
    }

    fn key(&mut self, ks: &gpui::Keystroke, cx: &mut Context<Self>) -> bool {
        let capturing_find = self.find.open && self.find.key.is_none();
        if !(self.capturing && self.selected.is_some()) && !capturing_find {
            return false;
        }
        if capturing_find && ks.key == "escape" && !ks.modifiers.modified() {
            self.capture.cancel();
            self.find = Find::default();
            cx.notify();
            return true;
        }
        let Some((code, mods)) = crate::native_gui::ui::keymap::console_key(ks) else { return true };
        let bound: Vec<String> = self.bindings.keys().cloned().collect();
        if let Captured::Name(name) = self.capture.push(code, mods, &bound, Instant::now()) {
            if capturing_find {
                self.find.matches = find_matches(&name, &self.bindings, &self.defaults);
                self.find.key = Some(name);
                self.find.at = 0;
                if let Some((a, _)) = self.find.matches.first() {
                    self.selected = Some(a.clone());
                    self.collapsed.clear();
                }
            } else {
                self.apply(name, cx);
            }
        }
        cx.notify();
        true
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        if let Ok(b) = serde_json::from_str(&self.saved) {
            self.bindings = b;
        }
        self.changed(cx);
        self.say("Reverted", cx);
    }

    fn reset_defaults(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let me = cx.entity().downgrade();
        let main = self.main.clone();
        dialogs::confirm(
            self,
            main,
            "Reset all keybindings to TinyFugue defaults?".into(),
            false,
            move |_w, cx| {
                let _ = me.update(cx, |this, cx| this.send(WsMessage::ResetKeybindDefaults, cx));
            },
            window,
            cx,
        );
    }

    fn list(&self, k: &Kit, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = &k.t;
        let mut col = div().flex().flex_col();
        for cat in ORDER {
            let actions: Vec<&ActionInfo> = self.actions.iter().filter(|a| a.category == cat).collect();
            if actions.is_empty() {
                continue;
            }
            let open = !self.collapsed.contains(cat);
            col = col.child(
                div()
                    .id(SharedString::from(format!("cat-{cat}")))
                    .flex()
                    .gap(px(6.0))
                    .px(px(10.0))
                    .py(px(6.0))
                    .text_size(px(11.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.muted)
                    .cursor_pointer()
                    .child(if open { "▼" } else { "▶" })
                    .child(cat.to_uppercase())
                    .on_click(cx.listener(move |this, _, _w, cx| {
                        if !this.collapsed.remove(cat) {
                            this.collapsed.insert(cat.to_string());
                        }
                        cx.notify();
                    })),
            );
            if !open {
                continue;
            }
            for a in actions {
                let keys = Self::keys_of(&self.bindings, &a.id);
                let selected = self.selected.as_deref() == Some(a.id.as_str());
                let found = self.find.matches.get(self.find.at).is_some_and(|(id, _)| *id == a.id);
                let hover = t.hover;
                let id = a.id.clone();
                let badges: Vec<gpui::AnyElement> = if keys.is_empty() {
                    vec![badge(k, "unbound", false, true)]
                } else {
                    keys.iter().map(|key| badge(k, key, self.modified(key), false)).collect()
                };
                col = col.child(
                    div()
                        .id(SharedString::from(format!("act-{}", a.id)))
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(8.0))
                        .px(px(10.0))
                        .py(px(4.0))
                        .cursor_pointer()
                        .when(selected, |d| d.bg(t.hover))
                        .when(found, |d| d.border_l_2().border_color(t.accent))
                        .when(!selected, |d| d.hover(move |s| s.bg(hover)))
                        .child(div().text_size(px(12.0)).min_w_0().overflow_hidden().whitespace_nowrap().child(a.name.clone()))
                        .child(div().flex().flex_wrap().justify_end().gap(px(3.0)).children(badges))
                        .on_click(cx.listener(move |this, _, _w, cx| {
                            this.selected = Some(id.clone());
                            this.capturing = false;
                            this.conflict = None;
                            this.capture.cancel();
                            cx.notify();
                        })),
                );
            }
        }
        col.into_any_element()
    }

    fn detail(&self, k: &Kit, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = &k.t;
        let Some(id) = self.selected.clone() else {
            return div().p(px(20.0)).text_color(t.muted).child("Select an action to edit its keybinding").into_any_element();
        };
        let action = self.actions.iter().find(|a| a.id == id).cloned().unwrap_or(ActionInfo { id: id.clone(), name: id.clone(), category: String::new() });
        let keys: Vec<String> = Self::keys_of(&self.bindings, &id).into_iter().map(str::to_string).collect();
        let defaults: Vec<String> = Self::keys_of(&self.defaults, &id).into_iter().map(str::to_string).collect();
        let removed: Vec<&String> = defaults.iter().filter(|d| !keys.contains(d)).collect();
        let rows = keys.iter().enumerate().map(|(i, key)| {
            let key_owned = key.clone();
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(badge(k, key, self.modified(key), false))
                .when(defaults.contains(key), |d| d.child(div().text_size(px(10.5)).text_color(t.muted).child("default")))
                .child(
                    div()
                        .id(("key-remove", i))
                        .px(px(6.0))
                        .text_color(t.error)
                        .cursor_pointer()
                        .child("×")
                        .on_click(cx.listener(move |this, _, _w, cx| {
                            if this.bindings.remove(&key_owned).is_some() {
                                this.changed(cx);
                            }
                        })),
                )
        });
        let conflict = self.conflict.clone().map(|(key, other)| {
            let name = self.name_of(&other);
            let (a, b) = (key.clone(), id.clone());
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(6.0))
                .p(px(8.0))
                .rounded(px(4.0))
                .bg(t.surface)
                .border_1()
                .border_color(t.highlight)
                .text_size(px(12.0))
                .child(div().font_weight(FontWeight::BOLD).child(key))
                .child(" is currently bound to ")
                .child(div().text_color(t.highlight).child(name))
                .child(".")
                .child(k.button("reassign", "Reassign", Btn::Plain, Some(Box::new(cx.listener(move |this, _, _w, cx| {
                    this.bindings.insert(a.clone(), b.clone());
                    this.capturing = false;
                    this.conflict = None;
                    this.changed(cx);
                })))).px(px(8.0)).py(px(2.0)))
                .child(k.button("reassign-cancel", "Cancel", Btn::Plain, Some(Box::new(cx.listener(|this, _, _w, cx| {
                    this.conflict = None;
                    cx.notify();
                })))).px(px(8.0)).py(px(2.0)))
        });
        let label = |text: &'static str| div().text_size(px(11.0)).font_weight(FontWeight::SEMIBOLD).text_color(t.muted).child(text.to_uppercase());
        let capturing = self.capturing;
        div()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .p(px(16.0))
            .child(div().text_size(px(16.0)).font_weight(FontWeight::BOLD).child(action.name.clone()))
            .child(div().text_size(px(11.5)).text_color(t.muted).child(format!("{} · {}", action.id, action.category)))
            .child(label("Current Bindings"))
            .child(if keys.is_empty() { div().text_size(px(12.0)).italic().text_color(t.fg2).child("No keys bound") } else { div().flex().flex_col().gap(px(4.0)).children(rows) })
            .when(!removed.is_empty(), |d| {
                d.child(div().flex().flex_wrap().items_center().gap(px(4.0)).text_size(px(11.0)).text_color(t.fg2).child("Default:").children(removed.iter().map(|r| badge(k, r, false, true))).child("(removed)"))
            })
            .child(label("Add Binding"))
            .child(
                div()
                    .id("capture")
                    .px(px(14.0))
                    .py(px(10.0))
                    .rounded(px(5.0))
                    .border_1()
                    .border_color(if capturing { t.accent } else { t.border })
                    .bg(if capturing { t.surface } else { k.field_bg })
                    .text_color(if capturing { t.accent } else { t.fg })
                    .cursor_pointer()
                    .child(if capturing { "Press a key combination..." } else { "Click to capture key" })
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.capturing = !this.capturing;
                        this.conflict = None;
                        this.capture.cancel();
                        window.focus(&this.focus, cx);
                        cx.notify();
                    })),
            )
            .children(conflict)
            .child(
                div()
                    .flex()
                    .gap(px(6.0))
                    .child(k.button("reset-action", "Reset to Default", Btn::Plain, Some(Box::new(cx.listener(|this, _, _w, cx| {
                        let Some(id) = this.selected.clone() else { return };
                        this.bindings.retain(|_, a| *a != id);
                        for (key, a) in this.defaults.clone() {
                            if a == id {
                                this.bindings.insert(key, a);
                            }
                        }
                        this.changed(cx);
                    })))))
                    .child(k.button("clear-action", "Clear All Bindings", Btn::Plain, Some(Box::new(cx.listener(|this, _, _w, cx| {
                        let Some(id) = this.selected.clone() else { return };
                        this.bindings.retain(|_, a| *a != id);
                        this.changed(cx);
                    }))))),
            )
            .into_any_element()
    }

    fn find_box(&self, k: &Kit, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if !self.find.open {
            return None;
        }
        let t = &k.t;
        let close = |cx: &mut Context<Self>| -> crate::native_gui::ui::kit::Click {
            Box::new(cx.listener(|this, _, _w, cx| {
                this.find = Find::default();
                this.capture.cancel();
                cx.notify();
            }))
        };
        let again = |cx: &mut Context<Self>| -> crate::native_gui::ui::kit::Click {
            Box::new(cx.listener(|this, _, _w, cx| {
                this.find.key = None;
                this.find.matches.clear();
                this.capture.cancel();
                cx.notify();
            }))
        };
        let mut body = div().flex().flex_col().gap(px(8.0)).child(div().font_weight(FontWeight::BOLD).child("Find Binding"));
        match &self.find.key {
            None => body = body.child(div().p(px(14.0)).rounded(px(5.0)).border_1().border_color(t.accent).text_color(t.accent).child("Press a key combination...")),
            Some(key) => {
                body = body.child(badge(k, key, false, false));
                match self.find.matches.get(self.find.at) {
                    None => body = body.child(div().text_size(px(12.0)).child("No bindings found for this key")),
                    Some((id, default_only)) => {
                        body = body.child(
                            div()
                                .flex()
                                .gap(px(4.0))
                                .text_size(px(12.0))
                                .child("Bound to:")
                                .child(div().font_weight(FontWeight::BOLD).child(self.name_of(id)))
                                .when(*default_only, |d| d.child("(default only)")),
                        );
                        if self.find.matches.len() > 1 {
                            body = body.child(div().text_size(px(12.0)).child(format!("{} of {} matches", self.find.at + 1, self.find.matches.len())));
                        }
                    }
                }
                let n = self.find.matches.len();
                let mut nav = div().flex().gap(px(6.0));
                if n > 1 {
                    for (id, label, by) in [("find-prev", "← Prev", n - 1), ("find-next", "Next →", 1)] {
                        nav = nav.child(k.button(id, label, Btn::Plain, Some(Box::new(cx.listener(move |this, _, _w, cx| {
                            this.find.at = (this.find.at + by) % n;
                            this.selected = this.find.matches.get(this.find.at).map(|m| m.0.clone());
                            cx.notify();
                        })))));
                    }
                }
                body = body.child(nav.child(k.button("find-again", "Search Again", Btn::Plain, Some(again(cx)))).child(k.button("find-close", "Close", Btn::Plain, Some(close(cx)))));
            }
        }
        Some(div().absolute().inset_0().flex().items_start().justify_center().pt(px(80.0)).bg(gpui::Hsla { a: 0.4, ..gpui::black() }).occlude().child(k.panel("find-binding").w(px(340.0)).child(body)).into_any_element())
    }
}

/// `.key-badge`: a key name (`modified`: not the default; `quiet`: unbound/removed).
fn badge(k: &Kit, key: &str, modified: bool, quiet: bool) -> gpui::AnyElement {
    let t = &k.t;
    div()
        .px(px(6.0))
        .py(px(1.0))
        .rounded(px(3.0))
        .border_1()
        .border_color(if modified { t.highlight } else { t.border })
        .bg(k.field_bg)
        .text_color(if quiet { t.muted } else if modified { t.highlight } else { t.fg })
        .font_family(k.fonts.primary.clone())
        .text_size(px(11.0))
        .when(quiet, |d| d.italic().opacity(0.7))
        .child(key.to_string())
        .into_any_element()
}

impl Host for KeybindEditor {
    fn dialogs(&mut self) -> &mut dialogs::Stack {
        &mut self.stack
    }

    fn restore_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
    }
}

impl EventEmitter<DialogEvent> for KeybindEditor {}
impl Focusable for KeybindEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for KeybindEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(k) = self.main.upgrade().map(|m| m.read(cx).kit()) else { return div().into_any_element() };
        let t = k.t.clone();
        let unsaved = self.loaded && self.snapshot() != self.saved;
        let status = self.status.as_ref().filter(|(_, at)| at.elapsed() < Duration::from_secs(2)).map(|(s, _)| s.clone());
        let pass = |key: &'static str| {
            move |this: &mut KeybindEditor, window: &mut Window, cx: &mut Context<KeybindEditor>| {
                // While capturing these are keys; otherwise the root's (Tab moves the focus).
                let Ok(ks) = gpui::Keystroke::parse(key) else { return };
                if !this.key(&ks, cx) {
                    cx.propagate();
                }
                let _ = window;
            }
        };
        let (tab, stab, cc, cmdc) = (pass("tab"), pass("shift-tab"), pass("ctrl-c"), pass("cmd-c"));
        let toolbar = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .px(px(12.0))
            .py(px(8.0))
            .border_b_1()
            .border_color(t.border)
            .child(div().font_weight(FontWeight::BOLD).child("Clay Keybind Editor"))
            .when(unsaved, |d| d.child(div().size(px(8.0)).rounded_full().bg(t.highlight)))
            .child(div().flex_1())
            .child(k.button("kb-find", "🔍", Btn::Plain, Some(Box::new(cx.listener(|this, _, window, cx| {
                this.find = Find { open: true, ..Default::default() };
                this.capture.cancel();
                window.focus(&this.focus, cx);
                cx.notify();
            })))).px(px(8.0)).py(px(3.0)))
            .child(k.button("kb-save", "Save", Btn::Primary, Some(Box::new(cx.listener(|this, _, _w, cx| this.send(WsMessage::SaveKeybindFile, cx))))).px(px(10.0)).py(px(3.0)))
            .child(k.button("kb-cancel", "Cancel", Btn::Plain, Some(Box::new(cx.listener(|this, _, _w, cx| this.cancel(cx))))).px(px(10.0)).py(px(3.0)))
            .child(k.button("kb-reset", "Reset Defaults", Btn::Danger, Some(Box::new(cx.listener(|this, _, w, cx| this.reset_defaults(w, cx))))).px(px(10.0)).py(px(3.0)))
            .children(status.map(|s| div().text_size(px(12.0)).text_color(t.success).child(s)));
        let find = self.find_box(&k, cx);
        div()
            .id("keybind-editor")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .font_family(k.fonts.ui.clone())
            .text_size(px(13.0))
            .text_color(t.fg)
            .on_action(cx.listener(move |this, _: &CapTab, w, cx| tab(this, w, cx)))
            .on_action(cx.listener(move |this, _: &CapShiftTab, w, cx| stab(this, w, cx)))
            .on_action(cx.listener(move |this, _: &CapCtrlC, w, cx| cc(this, w, cx)))
            .on_action(cx.listener(move |this, _: &CapCmdC, w, cx| cmdc(this, w, cx)))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _w, cx| {
                if this.key(&ev.keystroke, cx) {
                    cx.stop_propagation();
                }
            }))
            .child(toolbar)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(div().id("kb-list").w(px(380.0)).flex_shrink_0().overflow_y_scroll().border_r_1().border_color(t.border).py(px(4.0)).child(self.list(&k, cx)))
                    .child(div().id("kb-detail").flex_1().min_w_0().overflow_y_scroll().child(self.detail(&k, cx))),
            )
            .children(find)
            .children(self.stack.render())
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};

    fn press(c: &mut Capture, code: KeyCode, mods: KeyModifiers, bound: &[String]) -> Option<String> {
        match c.push(code, mods, bound, Instant::now()) {
            Captured::Name(n) => Some(n),
            _ => None,
        }
    }

    #[test]
    fn captures_name_keys_and_chords_as_the_page_does() {
        let mut c = Capture::default();
        assert_eq!(press(&mut c, KeyCode::Char('a'), KeyModifiers::CONTROL, &[]), Some("^A".into()));
        assert_eq!(press(&mut c, KeyCode::Char('a'), KeyModifiers::NONE, &[]), None, "a plain character isn't a key name");
        assert_eq!(press(&mut c, KeyCode::F(5), KeyModifiers::NONE, &[]), Some("F5".into()));
        // ^X always may start a chord.
        assert_eq!(press(&mut c, KeyCode::Char('x'), KeyModifiers::CONTROL, &[]), None);
        assert_eq!(press(&mut c, KeyCode::Char('r'), KeyModifiers::CONTROL, &[]), Some("^X^R".into()));
        // Escape then a key: Esc-b.
        assert_eq!(press(&mut c, KeyCode::Esc, KeyModifiers::NONE, &[]), None);
        assert_eq!(press(&mut c, KeyCode::Char('b'), KeyModifiers::NONE, &[]), Some("Esc-b".into()));
        // ^G cancels a chord.
        assert_eq!(press(&mut c, KeyCode::Char('x'), KeyModifiers::CONTROL, &[]), None);
        assert_eq!(press(&mut c, KeyCode::Char('g'), KeyModifiers::CONTROL, &[]), None);
        assert_eq!(press(&mut c, KeyCode::F(2), KeyModifiers::NONE, &[]), Some("F2".into()));
    }

    #[test]
    fn find_reports_current_and_default_only() {
        let b = BTreeMap::from([("^A".to_string(), "cursor_home".to_string())]);
        let d = BTreeMap::from([("^A".to_string(), "cursor_home".to_string()), ("^E".to_string(), "cursor_end".to_string())]);
        assert_eq!(find_matches("^A", &b, &d), vec![("cursor_home".into(), false)]);
        assert_eq!(find_matches("^E", &b, &d), vec![("cursor_end".into(), true)]);
        assert!(find_matches("^Q", &b, &d).is_empty());
    }
}
