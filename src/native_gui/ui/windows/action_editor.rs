//! The Action editor (src/web/action-editor.html), as a window of its own:
//! - the actions in a list, filtered by text and by world, each with its first pattern;
//! - the selected one's fields beside it: name, worlds, Enabled / Run at startup / GUI Menu
//!   Shortcut / Suppress Blanks, the match type, its patterns and its command;
//! - Save sends the whole list (`UpdateActions`), Cancel puts the action back as it was,
//!   Delete asks first.
//!
//! Edits go into the list as they're typed, so its names and patterns follow, as on the
//! page. Leaving an action with changes asks Save, Discard or Stay; so does closing the
//! window with anything unsaved. A list saved elsewhere replaces this one's unless this one
//! has unsaved changes.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use gpui::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Subscription, WeakEntity, Window, div, prelude::*, px, relative,
};
use gpui_component::checkbox::Checkbox;
use gpui_component::input::{InputEvent, InputState, Textarea, TextareaState};

use super::MainEvent;
use super::actions::problem;
use crate::actions::{Action, MatchPattern, MatchType};
use crate::native_gui::ui::ClayView;
use crate::native_gui::ui::dialogs::{self, DialogEvent, Host};
use crate::native_gui::ui::kit::{Btn, Kit, Pick};
use crate::websocket::WsMessage;

/// How long "Saved", "Cancelled" or "Updated from server" shows.
const STATUS_FOR: Duration = Duration::from_secs(2);

/// The actions the list shows, in the list's own order: `filter` in a name, world, pattern
/// or command, and `world` in the world (action-editor.html `filteredActions`).
pub fn shown(actions: &[Action], filter: &str, world: &str) -> Vec<usize> {
    let (f, w) = (filter.trim().to_lowercase(), world.trim().to_lowercase());
    (0..actions.len())
        .filter(|&i| {
            let a = &actions[i];
            let text = f.is_empty()
                || a.name.to_lowercase().contains(&f)
                || a.world.to_lowercase().contains(&f)
                || a.patterns.iter().any(|p| p.pattern.to_lowercase().contains(&f))
                || a.command.to_lowercase().contains(&f);
            text && (w.is_empty() || a.world.to_lowercase().contains(&w))
        })
        .collect()
}

/// A list row's second line.
pub fn hint(a: &Action) -> String {
    let first = a.patterns.first().map(|p| p.pattern.as_str()).filter(|p| !p.is_empty());
    match a.patterns.len() {
        0 => "manual-only (no patterns)".into(),
        1 => first.unwrap_or("(empty pattern)").into(),
        n => format!("{n} patterns: {}", first.unwrap_or("(empty)")),
    }
}

/// A new action's name: `new_action_` and the time in base 36, as the page names it.
fn new_name(now_ms: u128) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let (mut n, mut out) = (now_ms, Vec::new());
    loop {
        out.push(DIGITS[(n % 36) as usize]);
        n /= 36;
        if n == 0 {
            break;
        }
    }
    out.reverse();
    format!("new_action_{}", String::from_utf8_lossy(&out))
}

fn json<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_default()
}

/// Where to go once the changes are dealt with.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Leave {
    Select(usize),
    New,
    Close,
}

/// The selected action's fields.
struct Form {
    name: Entity<InputState>,
    world: Entity<InputState>,
    match_type: Pick,
    patterns: Vec<Entity<InputState>>,
    command: Entity<TextareaState>,
    _subs: Vec<Subscription>,
}

pub struct ActionEditor {
    main: WeakEntity<ClayView>,
    focus: FocusHandle,
    filter: Entity<InputState>,
    world_filter: Entity<InputState>,
    /// The list as edited here.
    actions: Vec<Action>,
    /// The list as last saved or received.
    saved: String,
    selected: Option<usize>,
    /// The selected action as last saved or selected: what Cancel goes back to.
    baseline: String,
    form: Option<Form>,
    /// Asking Save, Discard or Stay before going there.
    leaving: Option<Leave>,
    error: Option<String>,
    status: Option<(String, Instant)>,
    stack: dialogs::Stack,
    _subs: Vec<Subscription>,
}

impl ActionEditor {
    pub fn new(main: WeakEntity<ClayView>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter actions..."));
        let world_filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter world..."));
        let mut subs = Vec::new();
        for f in [&filter, &world_filter] {
            subs.push(cx.subscribe(f, |_, _, ev: &InputEvent, cx| {
                if let InputEvent::Change = ev {
                    cx.notify();
                }
            }));
        }
        let actions = main.upgrade().map(|m| m.read(cx).session.core.actions.clone()).unwrap_or_default();
        if let Some(m) = main.upgrade() {
            // Saved here or anywhere else.
            subs.push(cx.subscribe_in(&m, window, |this, m, ev: &MainEvent, window, cx| {
                if let MainEvent::Actions = ev {
                    let actions = m.read(cx).session.core.actions.clone();
                    this.received(actions, "Updated from server", window, cx);
                }
            }));
        }
        let _ = main.update(cx, |v, cx| v.send_message(WsMessage::RequestActionEditorState, cx));
        // Closing with changes asks first, as the page's beforeunload does.
        let me = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| me.update(cx, |this, cx| this.may_close(window, cx)).unwrap_or(true));
        let mut v = Self {
            main,
            focus: cx.focus_handle(),
            filter,
            world_filter,
            saved: String::new(),
            actions: Vec::new(),
            selected: None,
            baseline: String::new(),
            form: None,
            leaving: None,
            error: None,
            status: None,
            stack: Default::default(),
            _subs: subs,
        };
        v.adopt(actions, window, cx);
        v
    }

    /// The server's list (`ActionEditorState`).
    pub fn state(&mut self, actions_json: &str, window: &mut Window, cx: &mut Context<Self>) {
        let actions: Vec<Action> = serde_json::from_str(actions_json).unwrap_or_default();
        self.received(actions, "", window, cx);
    }

    /// A list from the server: taken unless this one has unsaved changes.
    fn received(&mut self, mut actions: Vec<Action>, say: &str, window: &mut Window, cx: &mut Context<Self>) {
        for a in &mut actions {
            a.normalize();
        }
        if self.unsaved() || json(&actions) == self.saved {
            return;
        }
        self.adopt(actions, window, cx);
        if !say.is_empty() {
            self.say(say, cx);
        }
    }

    /// Take `actions` as the saved list, keeping the selection by name.
    fn adopt(&mut self, mut actions: Vec<Action>, window: &mut Window, cx: &mut Context<Self>) {
        for a in &mut actions {
            a.normalize();
        }
        let name = self.selected.and_then(|i| self.actions.get(i)).map(|a| a.name.clone());
        self.saved = json(&actions);
        self.actions = actions;
        let at = name.and_then(|n| self.actions.iter().position(|a| a.name == n));
        self.select(at, window, cx);
    }

    fn unsaved(&self) -> bool {
        json(&self.actions) != self.saved
    }

    fn current_dirty(&self) -> bool {
        self.selected.and_then(|i| self.actions.get(i)).is_some_and(|a| json(a) != self.baseline)
    }

    fn say(&mut self, text: &str, cx: &mut Context<Self>) {
        self.status = Some((text.to_string(), Instant::now()));
        cx.notify();
        cx.spawn(async |this, cx| {
            cx.background_executor().timer(STATUS_FOR).await;
            let _ = this.update(cx, |_, cx| cx.notify());
        })
        .detach();
    }

    fn input(&self, value: &str, placeholder: &str, window: &mut Window, cx: &mut Context<Self>) -> (Entity<InputState>, Subscription) {
        let (value, placeholder) = (value.to_string(), placeholder.to_string());
        let f = cx.new(|cx| InputState::new(window, cx).default_value(value).placeholder(placeholder));
        let sub = cx.subscribe(&f, |this, _, ev: &InputEvent, cx| {
            if let InputEvent::Change = ev {
                this.sync(cx);
            }
        });
        (f, sub)
    }

    /// Select `at` (`None`: nothing), its fields in the form.
    fn select(&mut self, at: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = at.filter(|i| *i < self.actions.len());
        self.error = None;
        self.form = None;
        let Some(a) = self.selected.map(|i| self.actions[i].clone()) else {
            self.baseline.clear();
            cx.notify();
            return;
        };
        self.baseline = json(&a);
        let mut subs = Vec::new();
        let (name, s) = self.input(&a.name, "", window, cx);
        subs.push(s);
        let (world, s) = self.input(&a.world, "", window, cx);
        subs.push(s);
        let match_type = Pick::new(&[("Regexp", "Regexp"), ("Wildcard", "Wildcard")], if a.match_type == MatchType::Wildcard { "Wildcard" } else { "Regexp" }, window, cx);
        subs.push(cx.subscribe(&match_type.state, |this, _, _: &gpui_component::select::SelectEvent<Vec<SharedString>>, cx| this.sync(cx)));
        let mut patterns = Vec::new();
        for p in &a.patterns {
            let (f, s) = self.input(&p.pattern, "^pattern$", window, cx);
            patterns.push(f);
            subs.push(s);
        }
        let command = {
            let c = a.command.clone();
            cx.new(|cx| TextareaState::new(window, cx).rows(3).default_value(c))
        };
        subs.push(cx.subscribe(&command, |this, _, ev: &InputEvent, cx| {
            if let InputEvent::Change = ev {
                this.sync(cx);
            }
        }));
        self.form = Some(Form { name, world, match_type, patterns, command, _subs: subs });
        cx.notify();
    }

    /// The form's fields into the selected action (the checkboxes write it themselves).
    fn sync(&mut self, cx: &mut Context<Self>) {
        let (Some(i), Some(f)) = (self.selected, &self.form) else { return };
        let Some(a) = self.actions.get_mut(i) else { return };
        a.name = f.name.read(cx).value().to_string();
        a.world = f.world.read(cx).value().to_string();
        a.match_type = if f.match_type.value(cx) == "Wildcard" { MatchType::Wildcard } else { MatchType::Regexp };
        a.patterns = f.patterns.iter().map(|p| MatchPattern { pattern: p.read(cx).value().to_string(), compiled_regex: None }).collect();
        a.command = f.command.read(cx).value().to_string();
        cx.notify();
    }

    fn flag(&mut self, set: fn(&mut Action, bool), on: bool, cx: &mut Context<Self>) {
        if let Some(a) = self.selected.and_then(|i| self.actions.get_mut(i)) {
            set(a, on);
        }
        cx.notify();
    }

    fn add_pattern(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.form.is_none() {
            return;
        }
        let (f, s) = self.input("", "^pattern$", window, cx);
        f.update(cx, |f, cx| f.focus(window, cx));
        if let Some(form) = &mut self.form {
            form.patterns.push(f);
            form._subs.push(s);
        }
        self.sync(cx);
    }

    fn remove_pattern(&mut self, at: usize, cx: &mut Context<Self>) {
        if let Some(form) = &mut self.form {
            if at < form.patterns.len() {
                form.patterns.remove(at);
            }
        }
        self.sync(cx);
    }

    /// Go to `to`, asking first if the selected action has changes (closing: if anything
    /// does).
    fn go(&mut self, to: Leave, window: &mut Window, cx: &mut Context<Self>) {
        if to == Leave::Select(self.selected.unwrap_or(usize::MAX)) {
            return;
        }
        let ask = if to == Leave::Close { self.unsaved() || self.current_dirty() } else { self.current_dirty() };
        if ask {
            self.leaving = Some(to);
            window.focus(&self.focus, cx);
            cx.notify();
        } else {
            self.proceed(to, window, cx);
        }
    }

    fn proceed(&mut self, to: Leave, window: &mut Window, cx: &mut Context<Self>) {
        match to {
            Leave::Select(i) => self.select(Some(i), window, cx),
            Leave::New => {
                let ms = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis());
                let a = Action { name: new_name(ms), patterns: vec![MatchPattern { pattern: String::new(), compiled_regex: None }], ..Default::default() };
                self.actions.push(a);
                self.select(Some(self.actions.len() - 1), window, cx);
                if let Some(form) = &self.form {
                    form.name.update(cx, |f, cx| {
                        f.focus(window, cx);
                        f.select_all(window, cx);
                    });
                }
            }
            // The window's frame closes the window.
            Leave::Close => cx.emit(DialogEvent::Close),
        }
    }

    /// The window's close button.
    pub(crate) fn may_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.unsaved() || self.current_dirty() {
            self.go(Leave::Close, window, cx);
            return false;
        }
        true
    }

    /// Save: the whole list goes, as the page sends it. `false` when the action's name
    /// won't do.
    fn save(&mut self, cx: &mut Context<Self>) -> bool {
        if let Some(i) = self.selected {
            let name = self.actions[i].name.trim().to_string();
            if let Some(p) = problem(&self.actions, &name, Some(i)) {
                self.error = Some(p.into());
                cx.notify();
                return false;
            }
            self.actions[i].name = name;
            self.baseline = json(&self.actions[i]);
        }
        self.error = None;
        self.saved = json(&self.actions);
        let actions = self.actions.clone();
        let _ = self.main.update(cx, |v, cx| v.send_message(WsMessage::UpdateActions { actions }, cx));
        self.say("Saved", cx);
        true
    }

    /// Cancel: the action as it was.
    fn discard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(i) = self.selected else { return };
        if let Ok(a) = serde_json::from_str::<Action>(&self.baseline) {
            self.actions[i] = a;
        }
        self.select(Some(i), window, cx);
        self.say("Cancelled", cx);
    }

    /// The Save / Discard / Stay answer.
    fn answer(&mut self, choice: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(to) = self.leaving.take() else { return };
        match choice {
            "save" => {
                if self.save(cx) {
                    self.proceed(to, window, cx);
                }
            }
            "discard" => {
                self.discard(window, cx);
                self.proceed(to, window, cx);
            }
            _ => {}
        }
        cx.notify();
    }

    fn ask_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(i) = self.selected else { return };
        let me = cx.entity().downgrade();
        let main = self.main.clone();
        dialogs::confirm(
            self,
            main,
            format!("Delete action \"{}\"?", self.actions[i].name),
            false,
            move |window, cx| {
                let _ = me.update(cx, |this, cx| {
                    if i < this.actions.len() {
                        this.actions.remove(i);
                    }
                    this.select(None, window, cx);
                    // Deleted is saved: the page leaves it to the next Save (B28).
                    this.save(cx);
                });
            },
            window,
            cx,
        );
    }

    fn list(&self, k: &Kit, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = &k.t;
        let filtering = !self.filter.read(cx).value().trim().is_empty() || !self.world_filter.read(cx).value().trim().is_empty();
        let shown = shown(&self.actions, &self.filter.read(cx).value(), &self.world_filter.read(cx).value());
        if shown.is_empty() {
            let text = if filtering { "No matching actions" } else { "No actions yet — click + New Action" };
            return div().px(px(10.0)).py(px(12.0)).text_size(px(11.0)).italic().text_color(t.fg2).child(text).into_any_element();
        }
        let rows = shown.into_iter().map(|i| {
            let a = &self.actions[i];
            let selected = self.selected == Some(i);
            let hover = t.hover;
            div()
                .id(("action", i))
                .flex()
                .items_center()
                .gap(px(8.0))
                .min_h(px(38.0))
                .px(px(10.0))
                .py(px(5.0))
                .border_b_1()
                .border_color(t.border_subtle)
                .relative()
                .cursor_pointer()
                .map(|d| if selected { d.bg(t.hover).child(div().absolute().left_0().top_0().bottom_0().w(px(3.0)).bg(t.accent)) } else { d.hover(move |s| s.bg(hover)) })
                .child(div().w(px(14.0)).flex_shrink_0().text_center().text_size(px(11.0)).text_color(if a.enabled { t.success } else { t.error }).child(if a.enabled { "●" } else { "○" }))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(div().text_size(px(12.5)).whitespace_nowrap().overflow_hidden().text_ellipsis().child(if a.name.is_empty() { "(unnamed)".to_string() } else { a.name.clone() }))
                        .child(div().text_size(px(10.5)).text_color(t.fg2).font_family(k.fonts.primary.clone()).whitespace_nowrap().overflow_hidden().text_ellipsis().child(hint(a))),
                )
                .on_click(cx.listener(move |this, _, window, cx| this.go(Leave::Select(i), window, cx)))
        });
        div().flex().flex_col().children(rows).into_any_element()
    }

    fn detail(&self, k: &Kit, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = k.t.clone();
        let (Some(i), Some(form)) = (self.selected, &self.form) else {
            return div().mt(px(40.0)).text_center().italic().text_color(t.fg2).child("Select an action from the list to edit it").into_any_element();
        };
        let a = &self.actions[i];
        let mono = k.fonts.primary.clone();
        let label = |text: &'static str| div().text_size(px(11.0)).font_weight(FontWeight::SEMIBOLD).text_color(t.fg2).child(text.to_uppercase());
        let note = |text: &'static str| div().text_size(px(11.0)).italic().text_color(t.muted).child(text);
        let field = |l: &'static str, body: gpui::AnyElement| div().flex().flex_col().gap(px(4.0)).child(label(l)).child(body);
        let check = |id: &'static str, text: &'static str, on: bool, set: fn(&mut Action, bool), cx: &mut Context<Self>| {
            Checkbox::new(id).label(text).checked(on).on_click(cx.listener(move |this, on: &bool, _w, cx| this.flag(set, *on, cx)))
        };
        let pattern_rows = form.patterns.iter().enumerate().map(|(n, p)| {
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(div().flex_1().child(k.input(p).font_family(mono.clone())))
                .child(k.button(("pattern-remove", n), "✕", Btn::Danger, Some(Box::new(cx.listener(move |this, _, _w, cx| this.remove_pattern(n, cx))))).px(px(7.0)).py(px(3.0)))
        });
        let patterns = div()
            .flex()
            .flex_col()
            .gap(px(5.0))
            .p(px(6.0))
            .rounded(px(4.0))
            .border_1()
            .border_color(t.border)
            .bg(k.field_bg)
            .when(form.patterns.is_empty(), |d| d.child(div().p(px(4.0)).child(note("No patterns — action runs only via /name"))))
            .children(pattern_rows);
        div()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .child(field("Name", k.input(&form.name).font_family(mono.clone()).into_any_element()))
            .child(field("World", div().flex().flex_col().gap(px(2.0)).child(k.input(&form.world).font_family(mono.clone())).child(note("Comma-separated world names")).into_any_element()))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_x(px(20.0))
                    .gap_y(px(6.0))
                    .child(check("ae-enabled", "Enabled", a.enabled, |a, on| a.enabled = on, cx))
                    .child(check("ae-startup", "Run at startup", a.startup, |a, on| a.startup = on, cx))
                    .child(check("ae-shortcut", "GUI Menu Shortcut", a.gui_shortcut, |a, on| a.gui_shortcut = on, cx))
                    .child(
                        check("ae-blanks", "Suppress Blanks", a.suppress_blanks, |a, on| a.suppress_blanks = on, cx)
                            .tooltip("Also hide up to 3 blank lines immediately following a matched line"),
                    ),
            )
            .child(field("Match Type", div().w(px(200.0)).child(form.match_type.view()).into_any_element()))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(label("Match Patterns"))
                    .child(note("Action fires when ANY pattern matches; first match supplies $1..$9. Leave empty for manual-only (/name)."))
                    .child(patterns)
                    .child(
                        div().flex().child(
                            k.button("pattern-add", "+ Add Pattern", Btn::Plain, Some(Box::new(cx.listener(|this, _, window, cx| this.add_pattern(window, cx)))))
                                .text_size(px(11.0))
                                .px(px(8.0))
                                .py(px(3.0)),
                        ),
                    ),
            )
            .child(field(
                "Command",
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(Textarea::new(&form.command).w_full().bg(k.field_bg).text_color(t.fg).font_family(mono.clone()).text_size(px(13.0)))
                    .child(note("Semicolon-separated. $1..$9 from captures. /gag, /highlight, /echo, /notify supported."))
                    .into_any_element(),
            ))
            .child(k.error(self.error.as_deref()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .mt(px(6.0))
                    .pt(px(12.0))
                    .border_t_1()
                    .border_color(t.border)
                    .child(k.button("ae-delete", "Delete Action", Btn::Danger, Some(Box::new(cx.listener(|this, _, w, cx| this.ask_delete(w, cx))))))
                    .child(div().flex_1())
                    .child(k.button("ae-cancel", "Cancel", Btn::Plain, Some(Box::new(cx.listener(|this, _, w, cx| this.discard(w, cx))))))
                    .child(k.button(
                        "ae-save",
                        "Save",
                        Btn::Primary,
                        Some(Box::new(cx.listener(|this, _, _w, cx| {
                            this.save(cx);
                        }))),
                    )),
            )
            .into_any_element()
    }

    /// The Save / Discard / Stay question.
    fn leave_box(&self, k: &Kit, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        self.leaving?;
        let t = &k.t;
        let name = self.selected.filter(|_| self.current_dirty()).and_then(|i| self.actions.get(i)).map(|a| if a.name.is_empty() { "(unnamed)".to_string() } else { a.name.clone() });
        let mut text = div().flex().flex_wrap().justify_center().gap(px(4.0)).child("Unsaved changes");
        if let Some(name) = name {
            text = text.child("to").child(div().text_color(t.accent).child(format!("\"{name}\"")));
        }
        let button = |id: &'static str, label: &'static str, kind: Btn, cx: &mut Context<Self>| {
            k.button(id, label, kind, Some(Box::new(cx.listener(move |this, _, w, cx| this.answer(id, w, cx)))))
        };
        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui::Hsla { a: 0.55, ..gpui::black() })
                .occlude()
                .child(
                    k.panel("leave")
                        .max_w(px(380.0))
                        .items_center()
                        .child(text)
                        .child(div().flex().gap(px(8.0)).child(button("save", "Save", Btn::Primary, cx)).child(button("discard", "Discard", Btn::Danger, cx)).child(button("stay", "Stay", Btn::Plain, cx))),
                )
                .into_any_element(),
        )
    }
}

impl Host for ActionEditor {
    fn dialogs(&mut self) -> &mut dialogs::Stack {
        &mut self.stack
    }

    fn restore_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
    }
}

impl EventEmitter<DialogEvent> for ActionEditor {}
impl Focusable for ActionEditor {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.filter.read(cx).focus_handle(cx)
    }
}

impl Render for ActionEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(k) = self.main.upgrade().map(|m| m.read(cx).kit()) else { return div().into_any_element() };
        let t = k.t.clone();
        let status = self.status.as_ref().filter(|(_, at)| at.elapsed() < STATUS_FOR).map(|(s, _)| s.clone());
        let toolbar = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .px(px(12.0))
            .py(px(8.0))
            .border_b_1()
            .border_color(t.border)
            .child(div().font_weight(FontWeight::BOLD).text_color(t.accent).child("Clay Action Editor"))
            .when(self.unsaved(), |d| d.child(div().size(px(8.0)).rounded_full().bg(t.highlight)))
            .children(status.map(|s| div().text_size(px(11.0)).text_color(t.success).child(s)));
        let side = div()
            .w(px(280.0))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .border_r_1()
            .border_color(t.border)
            .child(div().flex().flex_col().gap(px(4.0)).p(px(8.0)).border_b_1().border_color(t.border).child(k.input(&self.filter)).child(k.input(&self.world_filter)))
            .child(
                div().flex().px(px(8.0)).py(px(6.0)).border_b_1().border_color(t.border).child(
                    k.button("ae-new", "+ New Action", Btn::Plain, Some(Box::new(cx.listener(|this, _, w, cx| this.go(Leave::New, w, cx)))))
                        .flex_1()
                        .text_size(px(12.0))
                        .py(px(4.0)),
                ),
            )
            .child(div().id("ae-list").flex_1().min_h_0().overflow_y_scroll().child(self.list(&k, cx)));
        let leave = self.leave_box(&k, cx);
        div()
            .id("action-editor")
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .font_family(k.fonts.ui.clone())
            .text_size(px(13.0))
            .line_height(relative(1.4))
            .text_color(t.fg)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                // Escape is Stay.
                if this.leaving.is_some() && ev.keystroke.key == "escape" {
                    this.answer("stay", window, cx);
                    cx.stop_propagation();
                }
            }))
            .child(toolbar)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(side)
                    .child(div().id("ae-detail").flex_1().min_w_0().overflow_y_scroll().px(px(20.0)).py(px(16.0)).child(self.detail(&k, cx))),
            )
            .children(leave)
            .children(self.stack.render())
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(name: &str, world: &str, pats: &[&str], command: &str) -> Action {
        Action {
            name: name.into(),
            world: world.into(),
            patterns: pats.iter().map(|p| MatchPattern { pattern: p.to_string(), compiled_regex: None }).collect(),
            command: command.into(),
            ..Default::default()
        }
    }

    #[test]
    fn the_list_filters_as_the_page_filters() {
        let a = vec![action("zap", "", &["^z"], "kill $1"), action("Alpha", "Mud", &[], ""), action("heal", "other,mud", &["hp low", "dying"], "")];
        assert_eq!(shown(&a, "", ""), vec![0, 1, 2], "the list's own order, not sorted");
        assert_eq!(shown(&a, "KILL", ""), vec![0], "the command too");
        assert_eq!(shown(&a, "", "mud"), vec![1, 2]);
        assert_eq!(shown(&a, "dying", "mud"), vec![2], "both filters");
    }

    #[test]
    fn rows_hint_at_their_patterns() {
        assert_eq!(hint(&action("a", "", &[], "")), "manual-only (no patterns)");
        assert_eq!(hint(&action("a", "", &[""], "")), "(empty pattern)");
        assert_eq!(hint(&action("a", "", &["^x"], "")), "^x");
        assert_eq!(hint(&action("a", "", &["^x", "y"], "")), "2 patterns: ^x");
        assert_eq!(hint(&action("a", "", &["", "y"], "")), "2 patterns: (empty)");
    }

    #[test]
    fn new_actions_are_named_as_the_page_names_them() {
        // `Date.now().toString(36)`.
        assert_eq!(new_name(0), "new_action_0");
        assert_eq!(new_name(1_760_000_000_000), "new_action_mgj6k3cw");
    }
}
