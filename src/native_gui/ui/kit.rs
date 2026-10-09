//! The parts dialogs and windows are built from, in Clay's theme: the web client's modal
//! styles (style.css `.modal-content`, `.modal-buttons`, `.popup-header`, `.btn-help`,
//! `.error`), with rounded corners and a shadow - the whole of the "nicer look" is in
//! here, so every dialog gets it the same.

use gpui::{
    App, BoxShadow, ClickEvent, Div, ElementId, Entity, FontWeight, Hsla, InteractiveElement, ParentElement, SharedString,
    Stateful, StatefulInteractiveElement, Styled, Window, div, point, px, relative,
};
use gpui_component::input::{Input, InputState};

use super::chrome::{MONO_LH, SANS_LH, Tokens, alpha};
use super::fonts::Fonts;
use super::output::hsla;

/// `.btn-primary`: its own cyan, not a theme colour, with dark text (and lighter on hover).
const PRIMARY: [u8; 3] = [0x08, 0x91, 0xb2];
const PRIMARY_HOVER: [u8; 3] = [0x22, 0xd3, 0xee];
const ON_PRIMARY: [u8; 3] = [0x08, 0x08, 0x0a];
/// `.btn-danger:hover`.
const DANGER_HOVER: [u8; 3] = [0xef, 0x44, 0x44];

#[derive(Clone)]
pub struct Kit {
    pub t: Tokens,
    pub fonts: Fonts,
    /// `--theme-bg`: what text fields sit on.
    pub field_bg: Hsla,
}

/// A button's kind (`.modal-buttons button`, `.btn-primary`, `.btn-danger`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Btn {
    Plain,
    Primary,
    Danger,
}

pub type Click = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

fn lighter(c: Hsla, by: f32) -> Hsla {
    Hsla { l: (c.l + by).min(1.0), ..c }
}

impl Kit {
    /// `.modal-content`: a dialog's box.
    pub fn panel(&self, id: impl Into<ElementId>) -> Stateful<Div> {
        div()
            .id(id)
            .flex()
            .flex_col()
            .gap(px(10.0))
            .p(px(20.0))
            .min_w(px(280.0))
            .bg(self.t.elevated)
            .border_1()
            .border_color(self.t.border)
            .rounded(px(8.0))
            .shadow(vec![BoxShadow {
                color: alpha([0, 0, 0], 0.45),
                offset: point(px(0.0), px(8.0)),
                blur_radius: px(24.0),
                spread_radius: px(0.0),
                inset: false,
            }])
            .font_family(self.fonts.ui.clone())
            .text_size(px(13.0))
            .line_height(relative(SANS_LH))
            .text_color(self.t.fg)
    }

    /// `.modal-content h2`: the title of the older dialogs, in the accent.
    pub fn heading(&self, text: impl Into<SharedString>) -> Div {
        div().text_size(px(16.0)).font_weight(FontWeight::BOLD).text_color(self.t.accent).child(text.into())
    }

    /// A title in a colour of its own (the certificate dialogs' red).
    pub fn heading_in(&self, text: impl Into<SharedString>, color: Hsla) -> Div {
        div().text_size(px(16.0)).font_weight(FontWeight::BOLD).text_color(color).child(text.into())
    }

    /// `.popup-header`: the title and the ✕ that closes.
    pub fn header(&self, id: impl Into<ElementId>, title: impl Into<SharedString>, close: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Div {
        let (surface, hover, muted, fg) = (self.t.surface, self.t.hover, self.t.muted, self.t.fg);
        div()
            .flex()
            .items_center()
            .justify_between()
            .child(div().text_size(px(16.0)).font_weight(FontWeight::BOLD).child(title.into()))
            .child(
                div()
                    .id(id)
                    .size(px(22.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.0))
                    .bg(surface)
                    .text_color(muted)
                    .text_size(px(13.0))
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover).text_color(fg))
                    .child("✕")
                    .on_click(close),
            )
    }

    /// Body text (`.modal-content p`).
    pub fn text(&self, text: impl Into<SharedString>) -> Div {
        div().text_color(self.t.fg).child(text.into())
    }

    /// Text with some of it bold, wrapping as one paragraph.
    pub fn rich(&self, parts: &[(&str, bool)]) -> gpui::StyledText {
        let mut text = String::new();
        let mut bold = Vec::new();
        for (part, b) in parts {
            let start = text.len();
            text.push_str(part);
            if *b {
                bold.push((start..text.len(), gpui::HighlightStyle { font_weight: Some(FontWeight::BOLD), ..Default::default() }));
            }
        }
        gpui::StyledText::new(text).with_highlights(bold)
    }

    /// Quieter text: explanations under a title.
    pub fn note(&self, text: impl Into<SharedString>) -> Div {
        div().text_color(self.t.fg2).child(text.into())
    }

    /// Monospace text that breaks anywhere (fingerprints, addresses).
    pub fn mono(&self, text: impl Into<SharedString>) -> Div {
        div().font_family(self.fonts.primary.clone()).text_size(px(12.0)).line_height(relative(MONO_LH)).child(text.into())
    }

    /// `.error`: a line kept for an error even when there is none, so the box doesn't jump.
    pub fn error(&self, text: Option<&str>) -> Div {
        div().min_h(px(17.0)).text_color(self.t.error).children(text.map(|t| t.to_string()))
    }

    /// `.modal-buttons`: buttons on the right (a Delete goes left with `.mr_auto()`).
    pub fn buttons(&self) -> Div {
        div().flex().items_center().justify_end().gap(px(6.0)).mt(px(4.0))
    }

    /// A button; `on_click` is left off when it's disabled.
    pub fn button(&self, id: impl Into<ElementId>, label: impl Into<SharedString>, kind: Btn, on_click: Option<Click>) -> Stateful<Div> {
        let t = &self.t;
        let (bg, fg, border) = match kind {
            Btn::Plain => (t.hover, t.fg, t.border),
            Btn::Primary => (hsla(PRIMARY), hsla(ON_PRIMARY), hsla(PRIMARY)),
            Btn::Danger => (t.error, gpui::white(), t.error),
        };
        let hover_bg = match kind {
            Btn::Plain => lighter(t.hover, 0.06),
            Btn::Primary => hsla(PRIMARY_HOVER),
            Btn::Danger => hsla(DANGER_HOVER),
        };
        let b = div()
            .id(id)
            .px(px(16.0))
            .py(px(6.0))
            .rounded(px(5.0))
            .border_1()
            .border_color(border)
            .bg(bg)
            .text_color(fg)
            .font_weight(if kind == Btn::Plain { FontWeight::NORMAL } else { FontWeight::SEMIBOLD })
            .child(label.into());
        match on_click {
            Some(f) => b.cursor_pointer().hover(move |s| s.bg(hover_bg).border_color(hover_bg)).on_click(f),
            None => b.opacity(0.45),
        }
    }

    /// `.btn-help`: the round "?" that explains the dialog it's in.
    pub fn help_button(&self, id: impl Into<ElementId>, on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Stateful<Div> {
        let (border, fg, hover) = (self.t.border, self.t.fg, self.t.hover);
        div()
            .id(id)
            .size(px(26.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .border_1()
            .border_color(border)
            .text_color(fg)
            .font_weight(FontWeight::BOLD)
            .text_size(px(14.0))
            .opacity(0.7)
            .cursor_pointer()
            .hover(move |s| s.opacity(1.0).bg(hover))
            .child("?")
            .on_click(on_click)
    }

    /// `.toggle`: a 32x18 switch, the knob right and the track cyan when on.
    pub fn toggle(&self, id: impl Into<ElementId>, on: bool, on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Stateful<Div> {
        let (track, border, knob) = if on { (hsla(PRIMARY), hsla(PRIMARY), hsla(ON_PRIMARY)) } else { (self.field_bg, self.t.border_subtle, self.t.muted) };
        div()
            .id(id)
            .w(px(32.0))
            .h(px(18.0))
            .flex_shrink_0()
            .relative()
            .rounded(px(9.0))
            .border_1()
            .border_color(border)
            .bg(track)
            .cursor_pointer()
            .child(div().absolute().top(px(2.0)).left(px(if on { 16.0 } else { 2.0 })).size(px(12.0)).rounded_full().bg(knob))
            .on_click(on_click)
    }

    /// A labelled text field: the label above, the field across the box.
    pub fn field(&self, label: impl Into<SharedString>, input: &Entity<InputState>) -> Div {
        div().flex().flex_col().gap(px(4.0)).child(div().text_color(self.t.fg2).child(label.into())).child(self.input(input))
    }

    /// A text field (`.modal-content input`).
    pub fn input(&self, input: &Entity<InputState>) -> Input {
        Input::new(input).w_full().bg(self.field_bg).text_color(self.t.fg).font_family(self.fonts.ui.clone()).text_size(px(13.0)).rounded(px(4.0))
    }
}

gpui::actions!(clay_list, [ListUp, ListDown, ListPageUp, ListPageDown, ListHome, ListEnd, ListEnter, ListEscape]);

/// The context a list's filter box sits in: its keys move the list, not the caret.
pub const LIST_CONTEXT: &str = "ClayList";

/// A list's keys while its filter box has the keyboard (a deeper context than the text
/// field's own; bound after gpui-component's, so they win). Home and End stay the box's.
pub fn bind_list_keys(cx: &mut App) {
    let at = Some("ClayList > Input");
    cx.bind_keys([
        gpui::KeyBinding::new("up", ListUp, at),
        gpui::KeyBinding::new("down", ListDown, at),
        gpui::KeyBinding::new("pageup", ListPageUp, at),
        gpui::KeyBinding::new("pagedown", ListPageDown, at),
        gpui::KeyBinding::new("enter", ListEnter, at),
        gpui::KeyBinding::new("escape", ListEscape, at),
    ]);
}

/// The next position in a list of `len` when moving by `by`, wrapping at the ends as the
/// web client's lists do (`None` when empty).
pub fn list_step(at: Option<usize>, len: usize, by: isize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let Some(at) = at.filter(|a| *a < len) else { return Some(if by < 0 { len - 1 } else { 0 }) };
    Some((at as isize + by).rem_euclid(len as isize) as usize)
}
