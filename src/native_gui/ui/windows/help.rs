//! The Help window (app.js `openHelpPopup`): the quick-start cards, then the full command
//! and keybinding reference, collapsed. The reference is read out of app.js's
//! `helpSections`, so the two clients list the same commands.

use std::collections::HashSet;

use gpui::{
    App, Context, EventEmitter, FocusHandle, Focusable, FontWeight, HighlightStyle, InteractiveElement, InteractiveText, IntoElement, KeyDownEvent,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, StyledText, WeakEntity, Window, div, prelude::*, px, relative, svg,
};

use crate::native_gui::ui::ClayView;
use crate::native_gui::ui::assets::icon;
use crate::native_gui::ui::chrome::MONO_LH;
use crate::native_gui::ui::dialogs::DialogEvent;
use crate::native_gui::ui::kit::Btn;

/// A piece of a card's text: plain, `<strong>`, `<kbd>`, or a link to a surface.
enum Seg {
    T(&'static str),
    B(&'static str),
    K(&'static str),
    L(&'static str, &'static str),
}

struct Card {
    icon: &'static str,
    title: &'static str,
    desc: &'static [Seg],
}

/// app.js `buildHelpQuickStartCards`, word for word.
const CARDS: &[Card] = &[
    Card { icon: "help-connect", title: "Connect to a world", desc: &[Seg::T("Tap the world name, or open "), Seg::B("World Selector"), Seg::T(" and hit "), Seg::B("Add"), Seg::T(" to set one up.")] },
    Card { icon: "help-switch", title: "Switch between worlds", desc: &[Seg::T("Tap the world name, swipe the tabs, or use "), Seg::K("▲"), Seg::T("/"), Seg::K("▼"), Seg::T(" at the bottom.")] },
    Card { icon: "help-send", title: "Type & send", desc: &[Seg::T("Type below and tap "), Seg::B("Send"), Seg::T(". Hold "), Seg::K("▲"), Seg::T("/"), Seg::K("▼"), Seg::T(" to reuse something you typed before.")] },
    Card { icon: "help-catchup", title: "Catch up on output", desc: &[Seg::B("MORE"), Seg::T(" means text is waiting, tap to reveal it. "), Seg::B("ACT"), Seg::T(" flags other busy worlds.")] },
    Card { icon: "help-actions", title: "Automate with Actions", desc: &[Seg::B("Actions"), Seg::T(" auto-respond to things the world sends. Pin one to the toolbar as a button.")] },
    Card {
        icon: "help-yours",
        title: "Make it yours",
        desc: &[Seg::T("Settings links to the "), Seg::L("Theme", "theme-editor"), Seg::T(" and "), Seg::L("Keybind", "keybind-editor"), Seg::T(" editors. The A\u{a0}—\u{a0}A slider resizes text.")],
    },
    Card { icon: "help-remote", title: "Use it from anywhere", desc: &[Seg::T("Turn on remote access in "), Seg::B("Settings → Web"), Seg::T(", and manage this device’s key there.")] },
    Card { icon: "help-notes", title: "Keep notes", desc: &[Seg::T("The notepad icon (when a world has notes) opens a simple editor for it.")] },
];

/// One collapsed group of the reference: its label and (left, right) rows.
#[derive(Debug, PartialEq)]
pub struct RefGroup {
    pub label: String,
    pub rows: Vec<(String, String)>,
}

/// As the browser shows it: entities decoded, runs of spaces collapsed.
fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The value of `key: '...'` on a line of app.js.
fn js_field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let at = line.find(&format!("{key}: '"))? + key.len() + 3;
    let rest = &line[at..];
    let mut end = 0;
    let bytes = rest.as_bytes();
    while end < bytes.len() {
        match bytes[end] {
            b'\\' => end += 2,
            b'\'' => return Some(&rest[..end]),
            _ => end += 1,
        }
    }
    None
}

/// app.js `helpSections`, grouped as `renderHelpReferenceHtml` groups it: Commands split
/// at its sub-headings, every other section one group (the second "Display" is "Function
/// Keys").
pub fn reference() -> Vec<RefGroup> {
    let js = crate::http::WEB_APP_JS;
    let Some(start) = js.find("const helpSections = [") else { return Vec::new() };
    let body = &js[start..];
    let body = &body[..body.find("\n    ];").unwrap_or(body.len())];
    let mut groups: Vec<RefGroup> = Vec::new();
    let mut in_commands = false;
    for line in body.lines().skip(1) {
        let line = line.trim();
        if let (Some(h), true) = (js_field(line, "heading"), line.contains("rows: [")) {
            in_commands = h == "Commands";
            if !in_commands {
                let label = if h == "Display" { "Function Keys".to_string() } else { unescape(h) };
                groups.push(RefGroup { label, rows: Vec::new() });
            }
        } else if let Some(h) = js_field(line, "heading") {
            if in_commands {
                groups.push(RefGroup { label: unescape(h), rows: Vec::new() });
            }
        } else if let (Some(l), Some(r)) = (js_field(line, "l"), js_field(line, "r")) {
            if let Some(g) = groups.last_mut() {
                g.rows.push((unescape(l), unescape(r)));
            }
        }
    }
    groups.retain(|g| !g.rows.is_empty());
    groups
}

pub struct HelpView {
    main: WeakEntity<ClayView>,
    focus: FocusHandle,
    show_reference: bool,
    open_groups: HashSet<usize>,
    groups: Vec<RefGroup>,
}

impl HelpView {
    pub fn new(main: WeakEntity<ClayView>, cx: &mut Context<Self>) -> Self {
        Self { main, focus: cx.focus_handle(), show_reference: false, open_groups: HashSet::new(), groups: reference() }
    }
}

impl EventEmitter<DialogEvent> for HelpView {}
impl Focusable for HelpView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for HelpView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(k) = self.main.upgrade().map(|m| m.read(cx).kit()) else { return div().into_any_element() };
        let t = k.t.clone();
        let link = self.main.upgrade().map(|m| m.read(cx).link_color()).unwrap_or(t.accent);
        let cards = CARDS.iter().enumerate().map(|(i, card)| {
            let mut text = String::new();
            let mut hl = Vec::new();
            let mut links = Vec::new();
            for seg in card.desc {
                let start = text.len();
                let (s, style) = match seg {
                    Seg::T(s) => (*s, None),
                    Seg::B(s) => (*s, Some(HighlightStyle { font_weight: Some(FontWeight::BOLD), color: Some(t.fg), ..Default::default() })),
                    Seg::K(s) => (*s, Some(HighlightStyle { color: Some(t.fg), background_color: Some(t.surface), ..Default::default() })),
                    Seg::L(s, target) => {
                        links.push((start..start + s.len(), *target));
                        (*s, Some(HighlightStyle { color: Some(link), underline: Some(gpui::UnderlineStyle { thickness: px(1.0), color: Some(link), wavy: false }), ..Default::default() }))
                    }
                };
                text.push_str(s);
                if let Some(style) = style {
                    hl.push((start..text.len(), style));
                }
            }
            let ranges: Vec<_> = links.iter().map(|(r, _)| r.clone()).collect();
            let targets: Vec<&'static str> = links.iter().map(|(_, t)| *t).collect();
            let main = self.main.clone();
            let desc = InteractiveText::new(("help-card-text", i), StyledText::new(text).with_highlights(hl)).on_click(ranges, move |ix, window, cx| {
                let target = targets[ix];
                let _ = main.update(cx, |v, cx| v.open_surface(target, window, cx));
            });
            div()
                .flex()
                .items_start()
                .gap(px(10.0))
                .py(px(10.0))
                .when(i + 1 < CARDS.len(), |d| d.border_b_1().border_color(t.border_subtle))
                .child(div().size(px(26.0)).flex_shrink_0().child(svg().path(icon(card.icon)).size(px(17.0)).text_color(t.accent)))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .min_w_0()
                        .child(div().font_weight(FontWeight::BOLD).text_color(t.fg).child(card.title))
                        .child(div().text_size(px(12.0)).line_height(relative(1.4)).text_color(t.fg2).line_clamp(2).child(desc)),
                )
        });
        let chevron = |open: bool| {
            svg().path(icon("help-chevron")).size(px(12.0)).text_color(t.muted).ml_auto().when(open, |s| s.with_transformation(gpui::Transformation::rotate(gpui::radians(std::f32::consts::PI))))
        };
        let groups = self.groups.iter().enumerate().map(|(i, g)| {
            let open = self.open_groups.contains(&i);
            let rows = g.rows.iter().map(|(l, r)| {
                div()
                    .flex()
                    .gap(px(10.0))
                    .py(px(3.0))
                    .text_size(px(11.5))
                    .child(div().flex_shrink_0().whitespace_nowrap().font_family(k.fonts.primary.clone()).line_height(relative(MONO_LH)).text_color(t.fg).child(SharedString::from(l.clone())))
                    .child(div().min_w_0().text_color(t.fg2).child(SharedString::from(r.clone())))
            });
            div()
                .border_t_1()
                .border_color(t.border_subtle)
                .child(
                    div()
                        .id(("help-ref-group", i))
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .py(px(8.0))
                        .px(px(2.0))
                        .cursor_pointer()
                        .text_size(px(11.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(t.muted)
                        .child(g.label.to_uppercase())
                        .child(chevron(open))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !this.open_groups.remove(&i) {
                                this.open_groups.insert(i);
                            }
                            cx.notify();
                        })),
                )
                .when(open, |d| d.child(div().flex().flex_col().pb(px(6.0)).children(rows)))
        });
        let title = format!("Help for Clay v{}", crate::VERSION);
        div()
            .id("help-window")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .p(px(14.0))
            .font_family(k.fonts.ui.clone())
            .text_size(px(13.0))
            .text_color(t.fg)
            .on_key_down(cx.listener(|_, ev: &KeyDownEvent, _w, cx| {
                if matches!(ev.keystroke.key.as_str(), "escape" | "enter") {
                    cx.emit(DialogEvent::Close);
                    cx.stop_propagation();
                }
            }))
            .child(k.header("help-close", title, cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))))
            .child(
                div()
                    .id("help-content")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .line_height(relative(1.4))
                    .py(px(4.0))
                    .child(div().flex().flex_col().children(cards))
                    .child(
                        div()
                            .mt(px(8.0))
                            .border_1()
                            .border_dashed()
                            .border_color(t.border)
                            .rounded(px(8.0))
                            .child(
                                div()
                                    .id("help-ref-toggle")
                                    .flex()
                                    .items_center()
                                    .gap(px(8.0))
                                    .px(px(12.0))
                                    .py(px(9.0))
                                    .cursor_pointer()
                                    .text_size(px(12.0))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(t.fg2)
                                    .child("Show full command & keybinding reference")
                                    .child(chevron(self.show_reference))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.show_reference = !this.show_reference;
                                        cx.notify();
                                    })),
                            )
                            .when(self.show_reference, |d| d.child(div().px(px(10.0)).pt(px(2.0)).pb(px(10.0)).children(groups))),
                    ),
            )
            .child(k.buttons().child(k.button("help-ok", "Ok", Btn::Primary, Some(Box::new(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close)))))))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reference_is_app_js_help_sections_grouped_as_the_web_groups_it() {
        let groups = reference();
        let labels: Vec<&str> = groups.iter().map(|g| g.label.as_str()).collect();
        assert_eq!(labels.first(), Some(&"Connection"), "{labels:?}");
        assert!(labels.contains(&"Lookup & Translation"), "entities decoded: {labels:?}");
        assert!(labels.contains(&"Function Keys"), "the second Display: {labels:?}");
        assert!(labels.contains(&"Output"));
        let connection = &groups[0];
        assert!(connection.rows.iter().any(|(l, _)| l == "/worlds <name>"), "{:?}", connection.rows);
        let rows: usize = groups.iter().map(|g| g.rows.len()).sum();
        assert!(rows > 60, "{rows} rows");
    }
}
