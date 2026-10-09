//! The Actions window (app.js `openActionsListPopup`, `openActionsEditorPopup`,
//! `openActionsConfirmPopup`):
//! - the list: filter box, Name/World/Pattern columns, arrow keys, double-click to edit;
//! - the editor in its place;
//! - Delete asks first.
//!
//! Saving sends the whole list back (`UpdateActions`), as the web client does. An edited
//! action starts from the stored one, so fields the editor doesn't show (its owner in
//! multiuser mode) survive.

use gpui::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Render,
    StatefulInteractiveElement, Styled, Subscription, WeakEntity, Window, div, prelude::*, px, relative,
};
use gpui_component::IndexPath;
use gpui_component::input::{InputEvent, InputState, Textarea, TextareaState};
use gpui_component::select::{Select, SelectState};

use super::MainEvent;
use crate::actions::{Action, MatchPattern, MatchType};
use crate::native_gui::ui::ClayView;
use crate::native_gui::ui::dialogs::{self, DialogEvent, Host};
use crate::native_gui::ui::kit::{self, Btn, Kit, ListDown, ListEnter, ListEscape, ListPageDown, ListPageUp, ListUp};
use crate::websocket::WsMessage;

type Choice = Entity<SelectState<Vec<&'static str>>>;

/// The actions the list shows, in its order: by name, narrowed by the window's world
/// (`/actions <world>`) and the filter box (name, world or any pattern).
pub fn shown(actions: &[Action], world: &str, filter: &str) -> Vec<usize> {
    let (world, filter) = (world.to_lowercase(), filter.to_lowercase());
    let mut out: Vec<usize> = (0..actions.len())
        .filter(|&i| {
            let a = &actions[i];
            if !world.is_empty() && !a.world.to_lowercase().contains(&world) {
                return false;
            }
            filter.is_empty()
                || a.name.to_lowercase().contains(&filter)
                || a.world.to_lowercase().contains(&filter)
                || a.patterns.iter().any(|p| p.pattern.to_lowercase().contains(&filter))
                || a.pattern.to_lowercase().contains(&filter)
        })
        .collect();
    out.sort_by_key(|&i| actions[i].name.to_lowercase());
    out
}

/// The list's Pattern column: the first pattern, "+N more", or "(manual)".
pub fn pattern_column(a: &Action) -> String {
    let pats: Vec<&str> = if a.patterns.is_empty() && !a.pattern.is_empty() { vec![a.pattern.as_str()] } else { a.patterns.iter().map(|p| p.pattern.as_str()).collect() };
    match pats.as_slice() {
        [] => "(manual)".into(),
        [one] => one.to_string(),
        [first, rest @ ..] => format!("{first} +{} more", rest.len()),
    }
}

/// app.js `validateAction`.
pub fn problem(actions: &[Action], name: &str, editing: Option<usize>) -> Option<&'static str> {
    if name.is_empty() {
        return Some("Name is required");
    }
    if actions.iter().enumerate().any(|(i, a)| a.name.eq_ignore_ascii_case(name) && Some(i) != editing) {
        return Some("An action with this name already exists");
    }
    let lower = name.to_lowercase();
    if crate::native_gui::core::input::internal_commands().iter().any(|c| c.trim_start_matches('/') == lower) {
        return Some("Cannot use internal command name");
    }
    None
}

struct Editor {
    /// The action being edited (`None`: a new one).
    index: Option<usize>,
    name: Entity<InputState>,
    world: Entity<InputState>,
    match_type: Choice,
    patterns: Vec<Entity<InputState>>,
    enabled: Choice,
    startup: Choice,
    shortcut: Choice,
    blanks: Choice,
    command: Entity<TextareaState>,
    error: Option<String>,
}

pub struct ActionsView {
    main: WeakEntity<ClayView>,
    filter: Entity<InputState>,
    /// `/actions <world>`: only that world's actions.
    world: String,
    selected: Option<usize>,
    editor: Option<Editor>,
    stack: dialogs::Stack,
    _subs: Vec<Subscription>,
}

fn yes_no(yes: bool, window: &mut Window, cx: &mut Context<ActionsView>, no_first: bool) -> Choice {
    let items = if no_first { vec!["No", "Yes"] } else { vec!["Yes", "No"] };
    let at = items.iter().position(|v| *v == if yes { "Yes" } else { "No" }).unwrap_or(0);
    cx.new(|cx| SelectState::new(items, Some(IndexPath::new(at)), window, cx))
}

fn chosen(c: &Choice, cx: &App) -> &'static str {
    c.read(cx).selected_value().copied().unwrap_or("")
}

impl ActionsView {
    pub fn new(main: WeakEntity<ClayView>, world: Option<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter actions..."));
        let mut subs = vec![cx.subscribe_in(&filter, window, |this, _, ev: &InputEvent, _w, cx| {
            if let InputEvent::Change = ev {
                // Keep the selection if it still shows, else the first that does.
                let list = this.list(cx);
                if !this.selected.is_some_and(|s| list.contains(&s)) {
                    this.selected = list.first().copied();
                }
                cx.notify();
            }
        })];
        if let Some(m) = main.upgrade() {
            subs.push(cx.subscribe(&m, |this, _, ev: &MainEvent, cx| {
                if let MainEvent::Actions = ev {
                    let n = this.actions(cx).len();
                    if this.selected.is_some_and(|s| s >= n) {
                        this.selected = n.checked_sub(1);
                    }
                }
                cx.notify();
            }));
        }
        let mut v = Self { main, filter, world: world.unwrap_or_default(), selected: None, editor: None, stack: Default::default(), _subs: subs };
        v.selected = v.list(cx).first().copied();
        v
    }

    /// `/actions <world>` again while the window is open.
    pub fn set_world(&mut self, world: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.world = world.unwrap_or_default();
        self.filter.update(cx, |f, cx| f.set_value("", window, cx));
        self.editor = None;
        self.selected = self.list(cx).first().copied();
        self.filter.update(cx, |f, cx| f.focus(window, cx));
        cx.notify();
    }

    fn actions(&self, cx: &App) -> Vec<Action> {
        self.main.upgrade().map(|m| m.read(cx).session.core.actions.clone()).unwrap_or_default()
    }

    fn list(&self, cx: &App) -> Vec<usize> {
        shown(&self.actions(cx), &self.world, &self.filter.read(cx).value())
    }

    fn step(&mut self, by: isize, cx: &mut Context<Self>) {
        let list = self.list(cx);
        let at = self.selected.and_then(|s| list.iter().position(|&i| i == s));
        self.selected = kit::list_step(at, list.len(), by).map(|p| list[p]);
        cx.notify();
    }

    fn send(&self, actions: Vec<Action>, cx: &mut App) {
        let _ = self.main.update(cx, |v, cx| v.send_message(WsMessage::UpdateActions { actions }, cx));
    }

    fn edit(&mut self, index: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let actions = self.actions(cx);
        let a = index.and_then(|i| actions.get(i)).cloned();
        let text = |v: &str, ph: &str, window: &mut Window, cx: &mut Context<Self>| {
            let (v, ph) = (v.to_string(), ph.to_string());
            cx.new(|cx| InputState::new(window, cx).default_value(v).placeholder(ph))
        };
        let new = a.is_none();
        let a = a.unwrap_or_default();
        let mut pats: Vec<String> = a.patterns.iter().map(|p| p.pattern.clone()).collect();
        if pats.is_empty() && !a.pattern.is_empty() {
            pats.push(a.pattern.clone());
        }
        if new {
            pats = vec![String::new()];
        }
        let match_at = if a.match_type == MatchType::Wildcard { 1 } else { 0 };
        let editor = Editor {
            index,
            name: text(&a.name, "", window, cx),
            world: text(&a.world, "(empty = all worlds)", window, cx),
            match_type: cx.new(|cx| SelectState::new(vec!["Regexp", "Wildcard"], Some(IndexPath::new(match_at)), window, cx)),
            patterns: pats.iter().map(|p| text(p, "^pattern$", window, cx)).collect(),
            enabled: yes_no(a.enabled, window, cx, false),
            startup: yes_no(a.startup, window, cx, true),
            shortcut: yes_no(a.gui_shortcut, window, cx, true),
            // A new action starts with No here (the web keeps the last one's: B26).
            blanks: yes_no(!new && a.suppress_blanks, window, cx, true),
            command: {
                let c = a.command.clone();
                cx.new(|cx| TextareaState::new(window, cx).rows(3).default_value(c).placeholder("Commands (semicolon-separated)"))
            },
            error: None,
        };
        editor.name.update(cx, |f, cx| f.focus(window, cx));
        self.editor = Some(editor);
        cx.notify();
    }

    fn close_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editor = None;
        self.filter.update(cx, |f, cx| f.focus(window, cx));
        cx.notify();
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ed) = &mut self.editor else { return };
        let mut actions = self.main.upgrade().map(|m| m.read(cx).session.core.actions.clone()).unwrap_or_default();
        let name = ed.name.read(cx).value().trim().to_string();
        if let Some(p) = problem(&actions, &name, ed.index) {
            ed.error = Some(p.into());
            cx.notify();
            return;
        }
        let mut a = ed.index.and_then(|i| actions.get(i)).cloned().unwrap_or_default();
        a.name = name;
        a.world = ed.world.read(cx).value().trim().to_string();
        a.match_type = if chosen(&ed.match_type, cx) == "Wildcard" { MatchType::Wildcard } else { MatchType::Regexp };
        a.patterns = ed
            .patterns
            .iter()
            .map(|p| p.read(cx).value().to_string())
            .filter(|p| !p.trim().is_empty())
            .map(|pattern| MatchPattern { pattern, compiled_regex: None })
            .collect();
        a.pattern.clear();
        a.command = ed.command.read(cx).value().to_string();
        a.enabled = chosen(&ed.enabled, cx) == "Yes";
        a.startup = chosen(&ed.startup, cx) == "Yes";
        a.gui_shortcut = chosen(&ed.shortcut, cx) == "Yes";
        a.suppress_blanks = chosen(&ed.blanks, cx) == "Yes";
        match ed.index {
            Some(i) if i < actions.len() => actions[i] = a,
            _ => {
                actions.push(a);
                self.selected = Some(actions.len() - 1);
            }
        }
        self.send(actions, cx);
        self.close_editor(window, cx);
    }

    /// Delete asks first (`Delete action 'x'?`).
    fn ask_delete(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let actions = self.actions(cx);
        let Some(a) = actions.get(index) else { return };
        let name = if a.name.is_empty() { "(unnamed)".to_string() } else { a.name.clone() };
        let me = cx.entity().downgrade();
        let main = self.main.clone();
        dialogs::confirm(
            self,
            main,
            format!("Delete action '{name}'?"),
            move |window, cx| {
                let _ = me.update(cx, |this, cx| {
                    let mut actions = this.actions(cx);
                    if index < actions.len() {
                        actions.remove(index);
                        this.selected = if actions.is_empty() { None } else { Some(index.min(actions.len() - 1)) };
                        this.send(actions, cx);
                    }
                    if this.editor.is_some() {
                        this.close_editor(window, cx);
                    }
                });
            },
            window,
            cx,
        );
    }

    fn help(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let main = self.main.clone();
        dialogs::open_help(self, main, key, window, cx);
    }

    fn list_view(&mut self, k: &Kit, window: &mut Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let _ = window;
        let t = k.t.clone();
        let actions = self.actions(cx);
        let list = self.list(cx);
        let mono = k.fonts.primary.clone();
        let col = |w: Option<f32>, text: String, color: gpui::Hsla| {
            let d = div().overflow_hidden().text_ellipsis().whitespace_nowrap().text_color(color).child(text);
            match w {
                Some(w) => d.w(px(w)).flex_shrink_0(),
                None => d.flex_1().min_w_0(),
            }
        };
        let body: gpui::AnyElement = if actions.is_empty() || list.is_empty() {
            div().p(px(8.0)).text_color(t.muted).child(if actions.is_empty() { "No actions defined." } else { "No matching actions." }).into_any_element()
        } else {
            let header = div()
                .flex()
                .gap(px(8.0))
                .px(px(8.0))
                .py(px(4.0))
                .border_b_1()
                .border_color(t.border)
                .child(col(Some(150.0), "Name".into(), t.accent))
                .child(col(Some(100.0), "World".into(), t.accent))
                .child(col(None, "Pattern".into(), t.accent));
            let rows = list.iter().map(|&i| {
                let a = &actions[i];
                let selected = self.selected == Some(i);
                let (fg, world_fg, pat_fg) = if selected { (gpui::black(), gpui::black(), gpui::black()) } else { (t.fg, t.muted, t.dim) };
                let hover = t.hover;
                let row = div()
                    .id(("action-row", i))
                    .flex()
                    .gap(px(8.0))
                    .px(px(8.0))
                    .py(px(4.0))
                    .border_b_1()
                    .border_color(t.border_subtle)
                    .cursor_pointer()
                    .child(col(Some(150.0), if a.name.is_empty() { "(unnamed)".into() } else { a.name.clone() }, fg))
                    .child(col(Some(100.0), if a.world.is_empty() { "(all)".into() } else { a.world.clone() }, world_fg))
                    .child(col(None, pattern_column(a), pat_fg))
                    .on_click(cx.listener(move |this, ev: &gpui::ClickEvent, window, cx| {
                        this.selected = Some(i);
                        if ev.click_count() >= 2 {
                            this.edit(Some(i), window, cx);
                        }
                        cx.notify();
                    }));
                if selected { row.bg(t.accent) } else { row.hover(move |s| s.bg(hover)) }
            });
            div().flex().flex_col().child(header).children(rows).into_any_element()
        };
        let has_sel = self.selected.is_some_and(|s| s < actions.len());
        let sel = self.selected;
        div()
            .key_context(kit::LIST_CONTEXT)
            .size_full()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .on_action(cx.listener(|this, _: &ListUp, _w, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &ListDown, _w, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &ListPageUp, _w, cx| this.step(-10, cx)))
            .on_action(cx.listener(|this, _: &ListPageDown, _w, cx| this.step(10, cx)))
            .on_action(cx.listener(|this, _: &ListEnter, window, cx| {
                if let Some(s) = this.selected.filter(|s| *s < this.actions(cx).len()) {
                    this.edit(Some(s), window, cx);
                }
            }))
            .on_action(cx.listener(|_, _: &ListEscape, _w, cx| cx.emit(DialogEvent::Close)))
            .child(k.header("actions-close", "Actions", cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(k.input(&self.filter).font_family(mono.clone()))
                    .when(!self.world.is_empty(), |d| d.child(div().text_color(t.accent).text_size(px(12.0)).child(format!("World: {}", self.world)))),
            )
            .child(
                div()
                    .id("actions-list")
                    .flex_1()
                    .min_h(px(80.0))
                    .overflow_y_scroll()
                    .border_1()
                    .border_color(t.border)
                    .bg(k.field_bg)
                    .font_family(mono)
                    .child(body),
            )
            .child(
                k.buttons()
                    .child(k.help_button("actions-help", cx.listener(|this, _, w, cx| this.help("actionsList", w, cx))))
                    .child(k.button(
                        "action-delete",
                        "Delete",
                        Btn::Danger,
                        sel.filter(|_| has_sel).map(|s| -> kit::Click { Box::new(cx.listener(move |this, _, w, cx| this.ask_delete(s, w, cx))) }),
                    ))
                    .child(div().flex_1())
                    .child(k.button("action-add", "Add", Btn::Plain, Some(Box::new(cx.listener(|this, _, w, cx| this.edit(None, w, cx))))))
                    .child(k.button(
                        "action-edit",
                        "Edit",
                        Btn::Plain,
                        sel.filter(|_| has_sel).map(|s| -> kit::Click { Box::new(cx.listener(move |this, _, w, cx| this.edit(Some(s), w, cx))) }),
                    ))
                    .child(k.button("action-ok", "Ok", Btn::Primary, Some(Box::new(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close)))))),
            )
            .into_any_element()
    }

    fn editor_view(&mut self, k: &Kit, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = k.t.clone();
        let mono = k.fonts.primary.clone();
        let Some(ed) = &self.editor else { return div().into_any_element() };
        let label = |text: &'static str| div().w(px(90.0)).flex_shrink_0().text_right().text_color(t.fg2).child(text);
        let row = |l: &'static str, field: gpui::AnyElement| div().flex().items_center().gap(px(10.0)).child(label(l)).child(div().w(px(350.0)).child(field));
        let select = |c: &Choice| Select::new(c).w(px(350.0)).into_any_element();
        let pattern_rows = ed.patterns.iter().enumerate().map(|(i, p)| {
            div()
                .flex()
                .items_center()
                .gap(px(4.0))
                .child(div().flex_1().child(k.input(p).font_family(k.fonts.primary.clone())))
                .child(k.button(("pattern-remove", i), "✕", Btn::Danger, Some(Box::new(cx.listener(move |this, _, _w, cx| {
                    if let Some(ed) = &mut this.editor {
                        if i < ed.patterns.len() {
                            ed.patterns.remove(i);
                        }
                    }
                    cx.notify();
                })))).px(px(6.0)).py(px(2.0)))
        });
        let patterns = div()
            .w(px(350.0))
            .flex()
            .flex_col()
            .gap(px(4.0))
            .when(ed.patterns.is_empty(), |d| d.child(div().text_size(px(11.0)).italic().text_color(t.muted).child("No patterns — action runs only via /name")))
            .children(pattern_rows)
            .child(
                div().child(
                    k.button("pattern-add", "+ Add Pattern", Btn::Plain, Some(Box::new(cx.listener(|this, _, window, cx| {
                        if let Some(ed) = &mut this.editor {
                            let f = cx.new(|cx| InputState::new(window, cx).placeholder("^pattern$"));
                            f.update(cx, |f, cx| f.focus(window, cx));
                            ed.patterns.push(f);
                        }
                        cx.notify();
                    }))))
                    .text_size(px(11.0))
                    .px(px(8.0))
                    .py(px(2.0)),
                ),
            );
        let title = if ed.index.is_some() { "Edit Action" } else { "New Action" };
        let editing = ed.index;
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                if ev.keystroke.key == "escape" {
                    this.close_editor(window, cx);
                    cx.stop_propagation();
                }
            }))
            .child(k.header("action-editor-close", title, cx.listener(|this, _, w, cx| this.close_editor(w, cx))))
            .child(
                div()
                    .id("action-editor-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .pr(px(16.0))
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(row("Name:", k.input(&ed.name).font_family(mono.clone()).into_any_element()))
                    .child(row("World:", k.input(&ed.world).font_family(mono.clone()).into_any_element()))
                    .child(row("Match Type:", select(&ed.match_type)))
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .gap(px(10.0))
                            .child(
                                div()
                                    .w(px(90.0))
                                    .flex_shrink_0()
                                    .text_right()
                                    .text_color(t.fg2)
                                    .child("Patterns:")
                                    .child(div().text_size(px(11.0)).opacity(0.7).child("(any match fires; first supplies $1..$9)")),
                            )
                            .child(patterns),
                    )
                    .child(row("Enabled:", select(&ed.enabled)))
                    .child(row("Startup:", select(&ed.startup)))
                    .child(row("GUI Menu Shortcut:", select(&ed.shortcut)))
                    .child(row("Suppress Blanks:", select(&ed.blanks)))
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .gap(px(10.0))
                            .child(label("Command:"))
                            .child(Textarea::new(&ed.command).w(px(350.0)).bg(k.field_bg).text_color(t.fg).font_family(k.fonts.primary.clone()).text_size(px(13.0))),
                    )
                    .child(k.error(ed.error.as_deref())),
            )
            .child(
                k.buttons()
                    .child(k.help_button("action-editor-help", cx.listener(|this, _, w, cx| this.help("actionEditor", w, cx))))
                    .when_some(editing, |d, i| d.child(k.button("action-editor-delete", "Delete", Btn::Danger, Some(Box::new(cx.listener(move |this, _, w, cx| this.ask_delete(i, w, cx)))))))
                    .child(div().flex_1())
                    .child(k.button("action-editor-page", "⊞ Editor", Btn::Plain, Some(Box::new(cx.listener(|this, _, w, cx| {
                        let _ = this.main.update(cx, |v, cx| v.open_surface("action-editor", w, cx));
                    })))))
                    .child(k.button("action-editor-cancel", "Cancel", Btn::Plain, Some(Box::new(cx.listener(|this, _, w, cx| this.close_editor(w, cx))))))
                    .child(k.button("action-save", "Save", Btn::Primary, Some(Box::new(cx.listener(|this, _, w, cx| this.save(w, cx)))))),
            )
            .into_any_element()
    }
}

impl Host for ActionsView {
    fn dialogs(&mut self) -> &mut dialogs::Stack {
        &mut self.stack
    }

    fn restore_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match &self.editor {
            Some(ed) => ed.name.update(cx, |f, cx| f.focus(window, cx)),
            None => self.filter.update(cx, |f, cx| f.focus(window, cx)),
        }
    }
}

impl EventEmitter<DialogEvent> for ActionsView {}
impl Focusable for ActionsView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.filter.read(cx).focus_handle(cx)
    }
}

impl Render for ActionsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(k) = self.main.upgrade().map(|m| m.read(cx).kit()) else { return div().into_any_element() };
        let content = if self.editor.is_some() { self.editor_view(&k, cx) } else { self.list_view(&k, window, cx) };
        div()
            .id("actions-window")
            .size_full()
            .relative()
            .p(px(12.0))
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

    fn action(name: &str, world: &str, pats: &[&str]) -> Action {
        Action {
            name: name.into(),
            world: world.into(),
            patterns: pats.iter().map(|p| MatchPattern { pattern: p.to_string(), compiled_regex: None }).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn the_list_is_sorted_and_filtered_as_the_web_lists_it() {
        let a = vec![action("zap", "", &["^z"]), action("Alpha", "Mud", &[]), action("heal", "other", &["hp low", "dying"])];
        assert_eq!(shown(&a, "", ""), vec![1, 2, 0]);
        assert_eq!(shown(&a, "mud", ""), vec![1], "the window's world");
        assert_eq!(shown(&a, "", "dyi"), vec![2], "any pattern");
        assert_eq!(pattern_column(&a[1]), "(manual)");
        assert_eq!(pattern_column(&a[2]), "hp low +1 more");
    }

    #[test]
    fn names_are_checked_as_the_web_checks_them() {
        let a = vec![action("heal", "", &[])];
        assert_eq!(problem(&a, "", None), Some("Name is required"));
        assert_eq!(problem(&a, "HEAL", None), Some("An action with this name already exists"));
        assert_eq!(problem(&a, "heal", Some(0)), None, "its own name");
        assert_eq!(problem(&a, "connect", None), Some("Cannot use internal command name"));
    }
}
