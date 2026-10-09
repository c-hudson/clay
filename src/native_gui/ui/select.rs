//! Selecting and copying output, following links, and the output's right-click menu
//! (the GUI's own, webview_gui.rs: Copy and Debug Selection).
//!
//! A selection is two places in the displayed text (line id, byte), so it stays on the
//! same text while lines arrive below or history is prepended above. What's copied is
//! what's shown: the ▶/🛢️/✨ prefixes and timestamps included, as the browser copies them.

use std::collections::HashMap;
use std::ops::Range;
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, Anchor, ClipboardItem, Context, FontWeight, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, ParentElement, Pixels, Point, Styled, Window, anchored, deferred, div, point, prelude::*, px, relative,
};
use unicode_segmentation::UnicodeSegmentation;

use super::ClayView;
use super::chrome::{MONO_LH, SANS_LH, alpha};
use super::output::Pos;
use crate::native_gui::core::style::StyledLine;

/// How long the Debug Selection dialog's Copy button says "Copied!".
const COPIED_FOR: Duration = Duration::from_millis(1500);

/// What a press selected, kept whole while the drag extends it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Unit {
    Char,
    Word,
    Line,
}

#[derive(Clone, Copy, Debug)]
pub struct Selection {
    /// Where it started: for a word or line, its start and end.
    anchor: (Pos, Pos),
    head: Pos,
    unit: Unit,
    pub dragging: bool,
}

#[derive(Default)]
pub struct SelectState {
    pub sel: Option<Selection>,
    /// The visible lines' ids in order, and each one's place, as of the last render.
    pub ids: Vec<u64>,
    pub order: HashMap<u64, usize>,
    pub hover_link: Option<(u64, Range<usize>)>,
    /// Where the button went down on a link (a click there opens it).
    press_link: Option<(u64, Range<usize>)>,
    /// The right-click menu, at this window point.
    pub menu: Option<Point<Pixels>>,
    /// The Debug Selection dialog's text.
    pub debug: Option<String>,
    copied_at: Option<Instant>,
    /// The mouse while dragging, for scrolling when it's past the top or bottom.
    pub drag_at: Option<Point<Pixels>>,
    /// Dragging a grep window's scrollbar: where on the thumb it was taken.
    bar_grab: Option<f32>,
}

/// The word around `byte` (Unicode word boundaries; a run of spaces is its own word).
pub fn word_at(text: &str, byte: usize) -> Range<usize> {
    for (i, w) in text.split_word_bound_indices() {
        if byte < i + w.len() {
            return i..i + w.len();
        }
    }
    text.len()..text.len()
}

/// The link containing `byte`: the run of link-styled text around it.
pub fn link_at(line: &StyledLine, byte: usize) -> Option<Range<usize>> {
    let mut spans: Vec<(Range<usize>, bool)> = Vec::new();
    let mut at = 0;
    for r in &line.runs {
        spans.push((at..at + r.len, r.link));
        at += r.len;
    }
    let k = spans.iter().position(|(s, link)| *link && s.contains(&byte))?;
    let (mut a, mut b) = (k, k);
    while a > 0 && spans[a - 1].1 {
        a -= 1;
    }
    while b + 1 < spans.len() && spans[b + 1].1 {
        b += 1;
    }
    Some(spans[a].0.start..spans[b].0.end)
}

/// Where a link goes: `www.` gets https:// (app.js `linkifyUrls`).
pub fn href(text: &str) -> String {
    if text.len() >= 4 && text[..4].eq_ignore_ascii_case("www.") { format!("https://{text}") } else { text.to_string() }
}

/// The selection as (start, end) in display order, given each visible line's place; `None`
/// when it's empty or its lines are gone.
fn span(sel: &Selection, order: &HashMap<u64, usize>) -> Option<(Pos, Pos)> {
    let key = |p: Pos| order.get(&p.id).map(|&i| (i, p.byte));
    let (a0, a1) = sel.anchor;
    let (ka0, ka1, kh) = (key(a0)?, key(a1)?, key(sel.head)?);
    let (start, end) = if kh < ka0 { (sel.head, a1) } else if kh > ka1 { (a0, sel.head) } else { (a0, a1) };
    (key(start)? < key(end)?).then_some((start, end))
}

/// Each line's selected bytes for a span, `len` giving a line's text length.
fn ranges(span: (Pos, Pos), ids: &[u64], order: &HashMap<u64, usize>, len: impl Fn(u64) -> usize) -> HashMap<u64, Range<usize>> {
    let (start, end) = span;
    let mut out = HashMap::new();
    let (i0, i1) = (order[&start.id], order[&end.id]);
    for (i, &id) in ids.iter().enumerate().take(i1 + 1).skip(i0) {
        let a = if i == i0 { start.byte } else { 0 };
        let b = if i == i1 { end.byte } else { len(id) + 1 };
        if a < b {
            out.insert(id, a..b);
        }
    }
    out
}

impl ClayView {
    fn styled_text(&self, id: u64) -> Option<std::sync::Arc<StyledLine>> {
        self.styled.get(&id).map(|(_, s)| s.clone())
    }

    /// The selection as (start, end), in display order.
    fn selection_span(&self) -> Option<(Pos, Pos)> {
        span(self.select.sel.as_ref()?, &self.select.order)
    }

    /// The selected bytes of each visible line (a line's end, its newline, counts as one
    /// byte past its text).
    pub(super) fn selection_ranges(&self) -> HashMap<u64, Range<usize>> {
        match self.selection_span() {
            Some(sp) => ranges(sp, &self.select.ids, &self.select.order, |id| self.styled_text(id).map_or(0, |s| s.text.len())),
            None => HashMap::new(),
        }
    }

    /// The selected text as shown, lines joined with newlines.
    pub(super) fn selected_text(&self) -> String {
        let ranges = self.selection_ranges();
        // The %textdiv divider is never copied.
        let mut ids: Vec<&u64> = ranges.keys().filter(|id| !super::textdiv::is_divider(**id)).collect();
        ids.sort_by_key(|id| self.select.order[id]);
        let mut parts = Vec::new();
        for id in ids {
            let r = &ranges[id];
            if let Some(s) = self.styled_text(*id) {
                let n = s.text.len();
                parts.push(s.text[r.start.min(n)..r.end.min(n)].to_string());
            }
        }
        parts.join("\n")
    }

    /// The selected lines as the server sent them, escape codes shown as `<esc>`
    /// (app.js `getDebugSelectionText`).
    fn raw_selection(&self) -> String {
        let ranges = self.selection_ranges();
        let Some(world) = self.session.core.current_world() else { return String::new() };
        world.lines.iter().filter(|l| ranges.contains_key(&l.id)).map(|l| l.text.replace('\x1b', "<esc>")).collect::<Vec<_>>().join("\n")
    }

    pub(super) fn has_selection(&self) -> bool {
        self.selection_span().is_some()
    }

    pub(super) fn copy_selection(&self, cx: &mut Context<Self>) -> bool {
        let text = self.selected_text();
        if text.is_empty() {
            return false;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        true
    }

    /// The visible lines' order changed (a render): keep the map, and drop a selection
    /// whose lines are gone.
    pub(super) fn note_visible(&mut self, ids: Vec<u64>) {
        self.select.order = ids.iter().enumerate().map(|(i, id)| (*id, i)).collect();
        self.select.ids = ids;
        if let Some(sel) = self.select.sel {
            let known = |p: Pos| self.select.order.contains_key(&p.id);
            if !(known(sel.anchor.0) && known(sel.anchor.1) && known(sel.head)) {
                self.select.sel = None;
            }
        }
    }

    pub(super) fn output_mouse_down(&mut self, ev: &MouseDownEvent, cx: &mut Context<Self>) {
        let (x, y) = (f32::from(ev.position.x), f32::from(ev.position.y));
        if ev.button == MouseButton::Right {
            self.select.menu = Some(ev.position);
            cx.notify();
            return;
        }
        if ev.button != MouseButton::Left {
            return;
        }
        self.select.menu = None;
        // A grep window's scrollbar: take the thumb where it was pressed, or, pressed
        // beside it, by its middle (the view jumps there).
        let bar = self.scroll.borrow().bar_at(x, y);
        if let Some(b) = bar {
            let grab = if y >= b.thumb_top && y < b.thumb_top + b.thumb_h { y - b.thumb_top } else { b.thumb_h / 2.0 };
            self.select.bar_grab = Some(grab);
            self.scroll.borrow_mut().drag_thumb_to(y - grab);
            cx.notify();
            return;
        }
        let (hit, on) = {
            let s = self.scroll.borrow();
            (s.hit(x, y), s.byte_at(x, y))
        };
        self.select.press_link = on.and_then(|p| self.styled_text(p.id).and_then(|s| link_at(&s, p.byte)).map(|r| (p.id, r)));
        let Some(pos) = hit else {
            self.select.sel = None;
            cx.notify();
            return;
        };
        let text = self.styled_text(pos.id).map(|s| s.text.clone()).unwrap_or_default();
        let (unit, a, b) = match ev.click_count {
            2 => {
                let w = word_at(&text, pos.byte);
                (Unit::Word, w.start, w.end)
            }
            n if n >= 3 => (Unit::Line, 0, text.len() + 1),
            _ => (Unit::Char, pos.byte, pos.byte),
        };
        let anchor = (Pos { id: pos.id, byte: a }, Pos { id: pos.id, byte: b });
        self.select.sel = Some(Selection { anchor, head: anchor.1, unit, dragging: true });
        self.select.drag_at = Some(ev.position);
        cx.notify();
    }

    pub(super) fn output_mouse_move(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        let (x, y) = (f32::from(ev.position.x), f32::from(ev.position.y));
        if let Some(grab) = self.select.bar_grab {
            if ev.pressed_button == Some(MouseButton::Left) {
                self.scroll.borrow_mut().drag_thumb_to(y - grab);
                cx.notify();
                return;
            }
            self.select.bar_grab = None;
        }
        let dragging = self.select.sel.is_some_and(|s| s.dragging);
        if dragging && ev.pressed_button == Some(MouseButton::Left) {
            self.select.drag_at = Some(ev.position);
            self.extend_selection(x, y);
            cx.notify();
            return;
        }
        if let Some(sel) = self.select.sel.as_mut() {
            sel.dragging = false;
        }
        let on = self.scroll.borrow().byte_at(x, y);
        let link = on.and_then(|p| self.styled_text(p.id).and_then(|s| link_at(&s, p.byte)).map(|r| (p.id, r)));
        if link != self.select.hover_link {
            self.select.hover_link = link;
            cx.notify();
        }
    }

    /// Move the selection's free end to the point (snapping to words or lines when the
    /// press selected one).
    fn extend_selection(&mut self, x: f32, y: f32) {
        let Some(mut sel) = self.select.sel else { return };
        let Some(pos) = self.scroll.borrow().hit(x, y) else { return };
        let before = match (self.select.order.get(&pos.id), self.select.order.get(&sel.anchor.0.id)) {
            (Some(&p), Some(&a)) => (p, pos.byte) < (a, sel.anchor.0.byte),
            _ => false,
        };
        let text = self.styled_text(pos.id).map(|s| s.text.clone()).unwrap_or_default();
        sel.head = match sel.unit {
            Unit::Char => pos,
            Unit::Word => {
                let w = word_at(&text, pos.byte);
                Pos { id: pos.id, byte: if before { w.start } else { w.end } }
            }
            Unit::Line => Pos { id: pos.id, byte: if before { 0 } else { text.len() + 1 } },
        };
        self.select.sel = Some(sel);
    }

    pub(super) fn output_mouse_up(&mut self, ev: &MouseUpEvent, cx: &mut Context<Self>) {
        if self.select.bar_grab.take().is_some() {
            return;
        }
        let Some(mut sel) = self.select.sel else { return };
        if !sel.dragging {
            return;
        }
        sel.dragging = false;
        self.select.sel = Some(sel);
        self.select.drag_at = None;
        if sel.unit == Unit::Char && sel.anchor.0 == sel.head {
            // A click, not a drag: no selection; on a link, follow it.
            self.select.sel = None;
            let (x, y) = (f32::from(ev.position.x), f32::from(ev.position.y));
            let on = self.scroll.borrow().byte_at(x, y);
            if let (Some((id, r)), Some(p)) = (self.select.press_link.take(), on) {
                if p.id == id && r.contains(&p.byte) {
                    if let Some(s) = self.styled_text(id) {
                        cx.open_url(&href(&s.text[r]));
                    }
                }
            }
        } else {
            // As a browser on Linux does, a selection is also the primary selection.
            #[cfg(any(target_os = "linux", target_os = "freebsd"))]
            {
                let text = self.selected_text();
                if !text.is_empty() {
                    cx.write_to_primary(ClipboardItem::new_string(text));
                }
            }
        }
        cx.notify();
    }

    /// While dragging past the top or bottom, the view scrolls along (from the clock).
    pub(super) fn drag_scroll(&mut self, cx: &mut Context<Self>) {
        let Some(at) = self.select.drag_at else { return };
        if !self.select.sel.is_some_and(|s| s.dragging) {
            return;
        }
        let (first, last) = {
            let s = self.scroll.borrow();
            (s.hits.first().map(|r| r.top), s.hits.last().map(|r| r.top + s.row_h))
        };
        let y = f32::from(at.y);
        let step = match (first, last) {
            (Some(top), _) if y < top => -1,
            (_, Some(bottom)) if y > bottom => 1,
            _ => return,
        };
        self.user_scroll(step);
        self.extend_selection(f32::from(at.x), y);
        cx.notify();
    }

    /// The right-click menu: Copy, Debug Selection.
    pub(super) fn context_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let at = self.select.menu?;
        let t = &self.chrome.tokens;
        let enabled = self.has_selection();
        let item = |id: &'static str, label: &'static str| {
            let (hover, fg) = (t.hover, t.fg);
            let d = div()
                .id(id)
                .px(px(11.0))
                .py(px(6.0))
                .rounded(px(4.0))
                .font_weight(FontWeight::MEDIUM)
                .child(label);
            if enabled { d.text_color(t.fg2).cursor_pointer().hover(move |s| s.bg(hover).text_color(fg)) } else { d.text_color(t.dim) }
        };
        let panel = div()
            .id("output-context-menu")
            .occlude()
            .flex()
            .flex_col()
            .min_w(px(150.0))
            .p(px(5.0))
            .bg(t.elevated)
            .border_1()
            .border_color(t.border)
            .rounded(px(8.0))
            .shadow(vec![gpui::BoxShadow {
                color: alpha([0, 0, 0], 0.3),
                offset: point(px(0.0), px(2.0)),
                blur_radius: px(8.0),
                spread_radius: px(0.0),
                inset: false,
            }])
            .font_family(self.fonts.ui.clone())
            .line_height(relative(SANS_LH))
            .text_size(px(12.5))
            .child(item("ctx-copy", "Copy").on_click(cx.listener(|this, _, _w, cx| {
                this.select.menu = None;
                this.copy_selection(cx);
                cx.notify();
            })))
            .child(item("ctx-debug", "Debug Selection").on_click(cx.listener(|this, _, _w, cx| {
                this.select.menu = None;
                if this.has_selection() {
                    this.select.debug = Some(this.raw_selection());
                    this.select.copied_at = None;
                }
                cx.notify();
            })))
            .on_mouse_down_out(cx.listener(|this, _: &MouseDownEvent, _w, cx| {
                this.select.menu = None;
                cx.notify();
            }));
        Some(deferred(anchored().position(at).anchor(Anchor::TopLeft).snap_to_window().child(panel)).with_priority(2).into_any_element())
    }

    /// The Debug Selection dialog: the raw lines, with Copy and Close.
    pub(super) fn debug_dialog(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let text = self.select.debug.clone()?;
        let t = &self.chrome.tokens;
        let vp = window.viewport_size();
        let copied = self.select.copied_at.is_some_and(|at| at.elapsed() < COPIED_FOR);
        let button = |id: &'static str, label: &'static str, primary: bool| {
            let (bg, hover) = if primary { (t.accent, t.accent) } else { (t.surface, t.hover) };
            div()
                .id(id)
                .px(px(16.0))
                .py(px(4.0))
                .rounded(px(4.0))
                .border_1()
                .border_color(t.border)
                .bg(bg)
                .text_color(t.fg)
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .child(label)
        };
        let copy_text = text.clone();
        let dialog = div()
            .id("debug-selection")
            .flex()
            .flex_col()
            .gap(px(8.0))
            .p(px(12.0))
            .min_w(px(300.0))
            .max_w(vp.width * 0.9)
            .max_h(vp.height * 0.8)
            .bg(t.elevated)
            .border_1()
            .border_color(t.border)
            .rounded(px(8.0))
            .font_family(self.fonts.ui.clone())
            .line_height(relative(SANS_LH))
            .text_size(px(13.0))
            .text_color(t.fg)
            .child(div().font_weight(FontWeight::BOLD).text_size(px(14.0)).child("Debug Selection"))
            .child(
                div()
                    .id("debug-selection-text")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p(px(8.0))
                    .rounded(px(4.0))
                    .bg(gpui::black())
                    .text_color(alpha([0xc0, 0xc0, 0xc0], 1.0))
                    .font_family(self.fonts.primary.clone())
                    .line_height(relative(MONO_LH))
                    .child(text),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.0))
                    .child(button("debug-copy", if copied { "Copied!" } else { "Copy" }, true).on_click(cx.listener(move |this, _, _w, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(copy_text.clone()));
                        this.select.copied_at = Some(Instant::now());
                        cx.notify();
                        let view = cx.entity().downgrade();
                        cx.spawn(async move |_, cx| {
                            cx.background_executor().timer(COPIED_FOR).await;
                            let _ = view.update(cx, |_, cx| cx.notify());
                        })
                        .detach();
                    })))
                    .child(button("debug-close", "Close", false).on_click(cx.listener(|this, _, _w, cx| {
                        this.select.debug = None;
                        cx.notify();
                    }))),
            )
            .on_mouse_down_out(cx.listener(|this, _: &MouseDownEvent, _w, cx| {
                this.select.debug = None;
                cx.notify();
            }));
        let backdrop = div().id("debug-backdrop").occlude().w(vp.width).h(vp.height).flex().items_center().justify_center().bg(alpha([0, 0, 0], 0.6)).child(dialog);
        Some(deferred(anchored().position(point(px(0.0), px(0.0))).child(backdrop)).with_priority(3).into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_gui::core::style::{Palette, StyleOptions, style_line};

    fn pos(id: u64, byte: usize) -> Pos {
        Pos { id, byte }
    }

    #[test]
    fn a_selection_spans_lines_in_display_order_whichever_way_it_was_dragged() {
        // Lines 30, 10, 20 shown in that order (ids aren't display order: history
        // prepended later gets newer ids).
        let ids = vec![30, 10, 20];
        let order: HashMap<u64, usize> = ids.iter().enumerate().map(|(i, id)| (*id, i)).collect();
        let len = |_| 5usize;
        let down = Selection { anchor: (pos(30, 2), pos(30, 2)), head: pos(20, 3), unit: Unit::Char, dragging: false };
        let r = ranges(span(&down, &order).unwrap(), &ids, &order, len);
        assert_eq!(r[&30], 2..6, "to the end, newline included");
        assert_eq!(r[&10], 0..6);
        assert_eq!(r[&20], 0..3);
        let up = Selection { anchor: (pos(20, 3), pos(20, 3)), head: pos(30, 2), unit: Unit::Char, dragging: false };
        assert_eq!(ranges(span(&up, &order).unwrap(), &ids, &order, len), r);
        // A click is no selection; a word stays whole when the drag goes back past it.
        let click = Selection { anchor: (pos(10, 1), pos(10, 1)), head: pos(10, 1), unit: Unit::Char, dragging: false };
        assert!(span(&click, &order).is_none());
        let word = Selection { anchor: (pos(10, 1), pos(10, 4)), head: pos(30, 0), unit: Unit::Word, dragging: false };
        assert_eq!(span(&word, &order), Some((pos(30, 0), pos(10, 4))));
        // A line that's gone ends it.
        let gone = Selection { anchor: (pos(99, 0), pos(99, 0)), head: pos(10, 1), unit: Unit::Char, dragging: false };
        assert!(span(&gone, &order).is_none());
    }

    #[test]
    fn words_links_and_hrefs() {
        let t = "go to www.example.org/x, then";
        assert_eq!(&t[word_at(t, 1)], "go");
        assert_eq!(&t[word_at(t, 2)], " ");
        let theme = crate::theme::ThemeColors::dark_default();
        let line = style_line(t, &Palette::from_theme(&theme), &StyleOptions::default());
        let at = line.text.find("example").unwrap();
        let r = link_at(&line, at).expect("a link");
        assert_eq!(&line.text[r.clone()], "www.example.org/x");
        assert_eq!(href(&line.text[r]), "https://www.example.org/x");
        assert_eq!(link_at(&line, 0), None);
        assert_eq!(href("https://a.b"), "https://a.b");
    }
}
