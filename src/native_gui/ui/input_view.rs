//! The input line (index.html `#input-container`): the world's prompt, then the text being
//! typed, drawn and edited here rather than by a widget, so the editing keys, the input
//! method and the prompt all work on the one model (`core::input::Editor`).
//!
//! As the GUI draws it: the container is the output's background with 2px 4px padding;
//! the prompt sits beside the text, centred; the text area is `input_height` rows of
//! 1.2em tall (style.css `#input`: line-height 1.0, 2px padding, a white caret), and
//! scrolls to keep the caret in view. A password prompt (`echo_masked`) shows a dot per
//! character.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, App, Bounds, ClipboardItem, Context, ElementInputHandler, EntityInputHandler, FocusHandle, IntoElement,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point, Styled, TextRun,
    UTF16Selection, UnderlineStyle, Window, canvas, div, fill, point, prelude::*, px, relative, size,
};
use unicode_segmentation::UnicodeSegmentation;

use super::ClayView;
use super::fonts::Fonts;
use super::output::{hsla, text_runs};
use crate::native_gui::core::style::{Rgb, Run, StyledLine, style_line};

/// The caret blinks (on and off this long) for a while after the last edit, then stays
/// on, as GTK's does - a caret blinking forever would redraw the window twice a second.
const BLINK: Duration = Duration::from_millis(500);
const BLINK_FOR: Duration = Duration::from_secs(10);
/// `#input`'s padding.
const PAD: f32 = 2.0;

/// One row of the text as last laid out: where it is, and each character boundary in it
/// (byte offset in the editor's text, window x), the row's end included.
#[derive(Clone, Debug, Default)]
pub struct InputRow {
    pub top: f32,
    pub stops: Vec<(usize, f32)>,
}

/// What the last frame laid out, for the mouse, the input method and the up/down keys.
#[derive(Default)]
pub struct InputLayout {
    /// The first row shown.
    pub scroll: usize,
    /// Every row, the hidden ones with their tops above or below the box.
    pub rows: Vec<InputRow>,
    pub row_h: f32,
    pub bounds: Bounds<Pixels>,
}

pub type InputHandle = Rc<RefCell<InputLayout>>;

impl InputLayout {
    /// The row holding text offset `at` (the one it ends, when it ends a row).
    fn row_of(&self, at: usize) -> Option<usize> {
        let fits = |r: &InputRow| r.stops.first().is_some_and(|s| s.0 <= at) && r.stops.last().is_some_and(|s| s.0 >= at);
        let k = self.rows.iter().position(fits)?;
        // At a wrap point the next row starts with it: prefer that row.
        match self.rows.get(k + 1) {
            Some(next) if next.stops.first().is_some_and(|s| s.0 == at) && self.rows[k].stops.last().is_some_and(|s| s.0 == at) => {
                Some(k + 1)
            }
            _ => Some(k),
        }
    }

    pub fn x_of(&self, at: usize) -> Option<f32> {
        let r = &self.rows[self.row_of(at)?];
        r.stops.iter().find(|s| s.0 == at).or(r.stops.last()).map(|s| s.1)
    }

    /// The text offset nearest window point (x, y).
    pub fn hit(&self, x: f32, y: f32) -> Option<usize> {
        let first = self.rows.first()?;
        let row = if y < first.top { first } else { self.rows.iter().rev().find(|r| y >= r.top).unwrap_or(first) };
        let k = row.stops.windows(2).position(|w| x < (w[0].1 + w[1].1) / 2.0).unwrap_or(row.stops.len().saturating_sub(1));
        row.stops.get(k).map(|s| s.0)
    }

    /// The offset one row up (or down) at the same x: `None` past the first (last) row.
    pub fn vertical(&self, at: usize, down: bool) -> Option<usize> {
        let k = self.row_of(at)?;
        let x = self.x_of(at)?;
        let target = if down { k.checked_add(1).filter(|&t| t < self.rows.len())? } else { k.checked_sub(1)? };
        let r = &self.rows[target];
        r.stops.iter().min_by(|a, b| (a.1 - x).abs().total_cmp(&(b.1 - x).abs())).map(|s| s.0)
    }
}

/// What a frame of the text area needs.
struct Field {
    text: String,
    cursor: usize,
    selection: Option<Range<usize>>,
    marked: Option<Range<usize>>,
    misspelled: Vec<Range<usize>>,
    masked: bool,
    font_size: f32,
    fonts: Fonts,
    fg: Rgb,
    selection_bg: Rgb,
    caret: bool,
    rows_shown: usize,
    layout: InputHandle,
    focus: FocusHandle,
    view: gpui::Entity<ClayView>,
}

/// One grapheme as drawn: its bytes in the text and in the drawn string.
struct Unit {
    text: Range<usize>,
    shown: Range<usize>,
}

/// The string drawn for `text`: a dot per character for a password, control characters
/// as their control pictures (`input::display_control_char`), line breaks kept.
fn drawn(text: &str, masked: bool) -> (String, Vec<Unit>) {
    let mut out = String::with_capacity(text.len());
    let mut units = Vec::new();
    for (i, g) in text.grapheme_indices(true) {
        let start = out.len();
        if g == "\n" || g == "\r\n" {
            out.push('\n');
        } else if masked {
            out.push('•');
        } else {
            out.extend(g.chars().map(crate::input::display_control_char));
        }
        units.push(Unit { text: i..i + g.len(), shown: start..out.len() });
    }
    (out, units)
}

fn plain_run(len: usize, fg: Rgb) -> Run {
    Run { len, fg, bg: None, bold: false, italic: false, underline: false, blink: false, link: false, cell: None, emoji: None }
}

/// Text runs for a row of the drawn string, the composing text underlined, misspelled
/// words with a red wavy line.
fn row_runs(row: &str, at: usize, base: &[TextRun], f: &Field, units: &[Unit]) -> Vec<TextRun> {
    // `base` covers the whole logical line; cut out this row's part.
    let mut runs: Vec<TextRun> = Vec::new();
    let mut pos = 0;
    for r in base {
        let (s, e) = (pos.max(at), (pos + r.len).min(at + row.len()));
        pos += r.len;
        if s < e {
            let mut sub = r.clone();
            sub.len = e - s;
            runs.push(sub);
        }
    }
    // Underlines, split at their edges, in the drawn string's offsets.
    let to_shown = |t: usize| units.iter().find(|u| u.text.start >= t).map_or_else(|| units.last().map_or(0, |u| u.shown.end), |u| u.shown.start);
    let mut marks: Vec<(Range<usize>, UnderlineStyle)> = Vec::new();
    if let Some(m) = &f.marked {
        marks.push((to_shown(m.start)..to_shown(m.end), UnderlineStyle { thickness: px(1.0), color: Some(hsla(f.fg)), wavy: false }));
    }
    if !f.masked {
        for w in &f.misspelled {
            marks.push((to_shown(w.start)..to_shown(w.end), UnderlineStyle { thickness: px(1.0), color: Some(hsla([0xe0, 0x40, 0x40])), wavy: true }));
        }
    }
    for (range, style) in marks {
        let (s, e) = (range.start.max(at), range.end.min(at + row.len()));
        if s >= e {
            continue;
        }
        let mut out = Vec::new();
        let mut p = at;
        for r in runs {
            let (rs, re) = (p, p + r.len);
            p = re;
            for (a, b, on) in [(rs, s.max(rs).min(re), false), (s.max(rs).min(re), e.max(rs).min(re), true), (e.max(rs).min(re), re, false)] {
                if a < b {
                    let mut piece = r.clone();
                    piece.len = b - a;
                    if on {
                        piece.underline = Some(style);
                    }
                    out.push(piece);
                }
            }
        }
        runs = out;
    }
    runs
}

fn paint_field(bounds: Bounds<Pixels>, f: Field, window: &mut Window, cx: &mut App) {
    window.handle_input(&f.focus, ElementInputHandler::new(bounds, f.view.clone()), cx);
    let fs = px(f.font_size);
    let row_h = f.font_size;
    let inner = Bounds::new(point(bounds.origin.x + px(PAD), bounds.origin.y + px(PAD)), size(bounds.size.width - px(2.0 * PAD), bounds.size.height - px(2.0 * PAD)));
    let width = f32::from(inner.size.width).max(1.0);
    let left = f32::from(inner.origin.x);
    let (shown, units) = drawn(&f.text, f.masked);
    let to_text = |s: usize| units.iter().find(|u| u.shown.start >= s).map_or(f.text.len(), |u| u.text.start);

    // Lay out each line, wrap it, and shape each row.
    struct Row {
        line: Arc<gpui::ShapedLine>,
        at: usize,
    }
    let mut rows: Vec<Row> = Vec::new();
    let mut line_start = 0;
    for line in shown.split('\n') {
        let styled = StyledLine { text: line.to_string(), runs: vec![plain_run(line.len(), f.fg)], ..Default::default() };
        let base = if line.is_empty() { Vec::new() } else { text_runs(&styled, &f.fonts, window) };
        let base: Vec<TextRun> = base.into_iter().map(|mut r| {
            r.color = hsla(f.fg);
            r
        }).collect();
        let layout = window.text_system().layout_line(line, fs, &base, None);
        let x = |i: usize| f32::from(layout.x_for_index(i));
        for r in crate::native_gui::core::wrap::wrap_rows(line, &[], width, 0.0, &x) {
            let text = &line[r.clone()];
            let runs = row_runs(text, r.start, &base, &f, &units);
            let shaped = window.text_system().shape_line(text.to_string().into(), fs, &runs, None);
            rows.push(Row { line: Arc::new(shaped), at: line_start + r.start });
        }
        line_start += line.len() + 1;
    }

    // Keep the caret's row in view.
    let mut layout = f.layout.borrow_mut();
    let caret_shown = units.iter().find(|u| u.text.start >= f.cursor).map_or(shown.len(), |u| u.shown.start);
    let caret_row = rows.iter().rposition(|r| r.at <= caret_shown && caret_shown <= r.at + r.line.text.len()).unwrap_or(0);
    let visible = ((f32::from(inner.size.height) / row_h).floor() as usize).max(1).min(f.rows_shown.max(1));
    if caret_row < layout.scroll {
        layout.scroll = caret_row;
    } else if caret_row >= layout.scroll + visible {
        layout.scroll = caret_row + 1 - visible;
    }
    layout.scroll = layout.scroll.min(rows.len().saturating_sub(1));
    let top0 = f32::from(inner.origin.y) - layout.scroll as f32 * row_h;
    layout.row_h = row_h;
    layout.bounds = bounds;
    layout.rows = rows
        .iter()
        .enumerate()
        .map(|(k, r)| {
            let text = r.line.text.as_ref();
            let mut stops: Vec<(usize, f32)> =
                text.char_indices().map(|(o, _)| (to_text(r.at + o), left + f32::from(r.line.x_for_index(o)))).collect();
            stops.push((to_text(r.at + text.len()), left + f32::from(r.line.x_for_index(text.len()))));
            stops.dedup_by_key(|s| s.0);
            InputRow { top: top0 + k as f32 * row_h, stops }
        })
        .collect();
    let rows_laid = layout.rows.clone();
    drop(layout);

    window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
        // The selection, under the text.
        if let Some(sel) = &f.selection {
            for r in &rows_laid {
                let (Some(first), Some(last)) = (r.stops.first(), r.stops.last()) else { continue };
                let (s, e) = (sel.start.max(first.0), sel.end.min(last.0));
                if s >= e {
                    continue;
                }
                let x = |at: usize| r.stops.iter().find(|st| st.0 >= at).map_or(last.1, |st| st.1);
                window.paint_quad(fill(Bounds::new(point(px(x(s)), px(r.top)), size(px(x(e) - x(s)), px(row_h))), hsla(f.selection_bg)));
            }
        }
        for (k, r) in rows.iter().enumerate() {
            let top = top0 + k as f32 * row_h;
            if top + row_h < f32::from(bounds.origin.y) || top > f32::from(bounds.origin.y + bounds.size.height) {
                continue;
            }
            let _ = r.line.paint(point(px(left), px(top)), px(row_h), gpui::TextAlign::Left, None, window, cx);
        }
        // The caret: 1px, white (`caret-color: #ffffff`).
        if f.caret {
            if let Some(r) = rows_laid.get(caret_row) {
                let x = r.stops.iter().find(|s| s.0 >= f.cursor).or(r.stops.last()).map_or(left, |s| s.1);
                window.paint_quad(fill(Bounds::new(point(px(x), px(r.top)), size(px(1.0), px(row_h))), gpui::white()));
            }
        }
    });
}

impl ClayView {
    /// Whether the caret shows now: on while typing, blinking for a while after, then on.
    fn caret_on(&self) -> bool {
        let idle = self.input.activity.elapsed();
        idle >= BLINK_FOR || (idle.as_millis() / BLINK.as_millis()).is_multiple_of(2)
    }

    /// The tick redraws while the caret blinks.
    pub(super) fn blink(&mut self, cx: &mut Context<Self>) {
        let on = self.caret_on();
        if on != self.input.caret_was {
            self.input.caret_was = on;
            cx.notify();
        }
    }

    /// Something was typed or edited: the caret shows, and blinks again from now, and the
    /// spelling is looked at again.
    pub(super) fn input_touched(&mut self, cx: &mut Context<Self>) {
        self.input.activity = Instant::now();
        self.input.caret_was = true;
        self.check_spelling();
        cx.notify();
    }

    /// Load the dictionary when spell checking is on (again when its path changes).
    pub(super) fn spell_settings(&mut self, enabled: bool, path: &str) {
        let want = enabled.then(|| path.to_string());
        if want == self.input.speller_source {
            return;
        }
        self.input.speller_source = want.clone();
        *self.input.speller.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.input.native = None;
        self.input.misspelled.clear();
        // The system's checker, unless the user chose a dictionary file.
        if want.as_deref() == Some("") {
            self.input.native = super::native_spell::Native::new();
            if self.input.native.is_some() {
                return;
            }
        }
        if let Some(path) = want {
            let slot = self.input.speller.clone();
            std::thread::spawn(move || {
                let checker = crate::spell::SpellChecker::new(&path);
                *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(checker));
            });
        }
    }

    fn speller(&self) -> Option<Arc<crate::spell::SpellChecker>> {
        self.input.speller.lock().ok().and_then(|s| s.clone())
    }

    fn check_spelling(&mut self) {
        let e = &self.editor;
        let found = if let Some(n) = &self.input.native {
            crate::native_gui::core::spell::misspelled(&e.text, e.cursor, &self.input.misspelled, &|w| n.is_valid(w))
        } else if let Some(sp) = self.speller() {
            crate::native_gui::core::spell::misspelled(&e.text, e.cursor, &self.input.misspelled, &|w| sp.is_valid(w))
        } else {
            return;
        };
        self.input.misspelled = found;
    }

    /// Whether the checker in use knows `word` (with none, every word is fine).
    fn word_is_valid(&self, word: &str) -> bool {
        if let Some(n) = &self.input.native {
            return n.is_valid(word);
        }
        self.speller().is_none_or(|sp| sp.is_valid(word))
    }

    /// What a misspelled word might have been, from whichever checker is in use.
    fn suggest(&self, word: &str) -> Option<Vec<String>> {
        if let Some(n) = &self.input.native {
            return Some(n.suggestions(word, 5));
        }
        self.speller().map(|sp| sp.suggestions(word, 5))
    }

    /// The input line: the prompt, then the text.
    pub(super) fn input_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let world = self.session.core.current_world();
        let prompt = world.map(|w| w.prompt.clone()).unwrap_or_default();
        let masked = world.is_some_and(|w| w.echo_masked);
        let mono = self.fonts.primary.clone();
        let rows = self.input.rows.clamp(1, 15);
        let height = rows as f32 * 1.2 * self.font_size + 2.0 * PAD;
        let field = Field {
            text: self.editor.text.clone(),
            cursor: self.editor.cursor,
            selection: self.editor.selection(),
            marked: self.editor.marked.clone(),
            misspelled: self.input.misspelled.clone(),
            masked,
            font_size: self.font_size,
            fonts: self.fonts.clone(),
            fg: self.palette.fg,
            selection_bg: [self.theme.selection_bg.r, self.theme.selection_bg.g, self.theme.selection_bg.b],
            caret: self.caret_on() && self.focus.is_focused(window),
            rows_shown: rows,
            layout: self.input.layout.clone(),
            focus: self.focus.clone(),
            view: cx.entity(),
        };
        let mut bar = div().flex().items_start().flex_shrink_0().px(px(4.0)).py(px(2.0)).bg(hsla(self.palette.bg)).font_family(mono.clone()).text_size(px(self.font_size)).line_height(relative(1.2)).text_color(hsla(self.palette.fg));
        // The prompt sits on the field's first row, at its height (the web centres it in
        // the whole input area, so a multi-row input showed it beside the second row).
        if !prompt.is_empty() {
            let styled = style_line(&prompt, &self.palette, &self.style);
            let prompt = div().flex_shrink_0().whitespace_nowrap().mt(px(PAD)).line_height(px(self.font_size));
            bar = bar.child(prompt.child(super::chrome::styled_text(&styled, &mono)));
        }
        bar.child(
            div()
                .id("input")
                .flex_1()
                .min_w_0()
                .h(px(height))
                .cursor_text()
                .child(canvas(|_, _, _| {}, move |bounds, _, window, cx| paint_field(bounds, field, window, cx)).size_full())
                .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &MouseDownEvent, _w, cx| this.input_mouse_down(ev, cx)))
                .on_mouse_down(MouseButton::Right, cx.listener(|this, ev: &MouseDownEvent, _w, cx| this.input_menu_at(ev, cx)))
                .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _w, cx| this.input_mouse_move(ev, cx)))
                .on_mouse_up(MouseButton::Left, cx.listener(|this, _: &MouseUpEvent, _w, _cx| this.input.dragging = false)),
        )
        .into_any_element()
    }

    fn input_mouse_down(&mut self, ev: &MouseDownEvent, cx: &mut Context<Self>) {
        let at = self.input.layout.borrow().hit(f32::from(ev.position.x), f32::from(ev.position.y));
        let Some(at) = at else { return };
        self.input.menu = None;
        // A click in the input ends a selection in the output (one selection at a time).
        self.select.sel = None;
        match ev.click_count {
            2 => {
                let w = super::select::word_at(&self.editor.text, at);
                self.editor.set_cursor(w.start, false);
                self.editor.set_cursor(w.end, true);
            }
            n if n >= 3 => self.editor.select_all(),
            _ => self.editor.set_cursor(at, ev.modifiers.shift),
        }
        self.input.dragging = true;
        self.input_touched(cx);
    }

    /// ^Q (`spell_check`): a misspelled word at the cursor gets its suggestions, in the
    /// input's menu at the caret. The console prints them and cycles them in; the web's
    /// ^Q does nothing (B25). A word spelled right, or with none, does nothing either.
    pub(super) fn spell_suggest(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let at = self.editor.cursor;
        let Some(w) = crate::native_gui::core::spell::word_at(&self.editor.text, at) else { return };
        let word = self.editor.text[w.clone()].to_string();
        if self.word_is_valid(&word) {
            return;
        }
        let Some(list) = self.suggest(&word).filter(|l| !l.is_empty()) else { return };
        let layout = self.input.layout.borrow();
        let Some(row) = layout.row_of(at) else { return };
        let x = layout.x_of(at).unwrap_or(f32::from(layout.bounds.origin.x));
        let y = layout.rows[row].top + layout.row_h;
        drop(layout);
        self.input.menu = Some((point(px(x), px(y)), Some((w, list))));
        self.select.menu = None;
        cx.notify();
    }

    /// Right-click in the input: Cut, Copy, Paste, Select All, and for a misspelled word
    /// its suggestions first (as a browser's text field menu has).
    fn input_menu_at(&mut self, ev: &MouseDownEvent, cx: &mut Context<Self>) {
        let at = self.input.layout.borrow().hit(f32::from(ev.position.x), f32::from(ev.position.y));
        let word = at.and_then(|at| self.input.misspelled.iter().find(|w| w.start <= at && at <= w.end).cloned());
        let suggest = word.and_then(|w| Some((w.clone(), self.suggest(&self.editor.text[w])?)));
        // A right-click outside the selection moves the cursor there.
        if let Some(at) = at {
            if !self.editor.selection().is_some_and(|s| s.contains(&at)) {
                self.editor.set_cursor(at, false);
            }
        }
        self.input.menu = Some((ev.position, suggest));
        self.select.menu = None;
        cx.notify();
    }

    /// The input's right-click menu.
    pub(super) fn input_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (at, suggest) = self.input.menu.clone()?;
        let t = &self.chrome.tokens;
        let has_sel = self.editor.selection().is_some();
        let item = |id: gpui::SharedString, label: String, enabled: bool| {
            let (hover, fg) = (t.hover, t.fg);
            let d = div().id(id).px(px(11.0)).py(px(6.0)).rounded(px(4.0)).font_weight(gpui::FontWeight::MEDIUM).child(label);
            if enabled { d.text_color(t.fg2).cursor_pointer().hover(move |s| s.bg(hover).text_color(fg)) } else { d.text_color(t.dim) }
        };
        let mut panel = div()
            .id("input-context-menu")
            .occlude()
            .flex()
            .flex_col()
            .min_w(px(150.0))
            .p(px(5.0))
            .bg(t.elevated)
            .border_1()
            .border_color(t.border)
            .rounded(px(8.0))
            .font_family(self.fonts.ui.clone())
            .line_height(relative(super::chrome::SANS_LH))
            .text_size(px(12.5));
        if let Some((range, words)) = suggest {
            if words.is_empty() {
                panel = panel.child(item("spell-none".into(), "No suggestions".into(), false));
            }
            for (k, w) in words.into_iter().enumerate() {
                let r = range.clone();
                panel = panel.child(item(format!("spell-{k}").into(), w.clone(), true).on_click(cx.listener(move |this, _, _w, cx| {
                    this.input.menu = None;
                    this.editor.set_cursor(r.start, false);
                    this.editor.set_cursor(r.end, true);
                    this.editor.insert(&w);
                    this.input_touched(cx);
                })));
            }
            panel = panel.child(div().h(px(1.0)).mx(px(8.0)).my(px(4.0)).bg(t.border));
        }
        panel = panel
            .child(item("in-cut".into(), "Cut".into(), has_sel).on_click(cx.listener(|this, _, _w, cx| {
                this.input.menu = None;
                this.copy_input(true, cx);
            })))
            .child(item("in-copy".into(), "Copy".into(), has_sel).on_click(cx.listener(|this, _, _w, cx| {
                this.input.menu = None;
                this.copy_input(false, cx);
                cx.notify();
            })))
            .child(item("in-paste".into(), "Paste".into(), true).on_click(cx.listener(|this, _, _w, cx| {
                this.input.menu = None;
                this.paste(cx);
            })))
            .child(item("in-select-all".into(), "Select All".into(), !self.editor.text.is_empty()).on_click(cx.listener(|this, _, _w, cx| {
                this.input.menu = None;
                this.editor.select_all();
                this.input_touched(cx);
            })))
            .on_mouse_down_out(cx.listener(|this, _: &MouseDownEvent, _w, cx| {
                this.input.menu = None;
                cx.notify();
            }));
        Some(gpui::deferred(gpui::anchored().position(at).anchor(gpui::Anchor::BottomLeft).snap_to_window().child(panel)).with_priority(2).into_any_element())
    }

    fn input_mouse_move(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        if !self.input.dragging || ev.pressed_button != Some(MouseButton::Left) {
            self.input.dragging = false;
            return;
        }
        if let Some(at) = self.input.layout.borrow().hit(f32::from(ev.position.x), f32::from(ev.position.y)) {
            self.editor.set_cursor(at, true);
        }
        self.input_touched(cx);
    }

    /// Text arrived for the input (typed, pasted, or committed by the input method): what
    /// app.js's `input` event does after it.
    pub(super) fn typed(&mut self, cx: &mut Context<Self>) {
        self.kbnum = None;
        self.kbnum_sign = 1;
        if self.session.core.settings.as_ref().is_some_and(|s| s.temp_convert_enabled) {
            self.editor.convert_temperature();
        }
        self.input_touched(cx);
    }

    pub(super) fn paste(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
            self.editor.insert(&text);
            self.typed(cx);
        }
    }

    pub(super) fn copy_input(&mut self, cut: bool, cx: &mut Context<Self>) -> bool {
        let Some(text) = self.editor.selected_text().map(str::to_string) else { return false };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        if cut {
            self.editor.insert("");
            self.input_touched(cx);
        }
        true
    }
}

impl ClayView {
    /// Enter: send what's typed (app.js `sendCommand`).
    pub(super) fn send_line(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // /import collects its password here, never sending it as a command line
        // (app.js sendCommand).
        let trimmed = self.editor.text.trim().to_string();
        if trimmed == "/import" || trimmed.starts_with("/import ") {
            let args: Vec<&str> = trimmed[7..].split_whitespace().collect();
            let addr = match args.as_slice() {
                [host, port, ..] => format!("{host}:{port}"),
                [host] => host.to_string(),
                [] => String::new(),
            };
            self.editor.remember(&trimmed, false);
            self.editor.clear();
            self.open_import(&addr, window, cx);
            return;
        }
        if !self.session.core.is_online() {
            let command = self.editor.text.clone();
            self.editor.remember(&command, false);
            self.editor.clear();
            self.connection_lost(command, window, cx);
            return;
        }
        // Following the output: held lines come out and more-mode counts afresh. Scrolled
        // back: the place is kept, and the reply queues below.
        if self.scroll.borrow().at_bottom() {
            if self.more.paused {
                self.release_all(cx);
            }
            self.more.caught_up();
        }
        let core = &self.session.core;
        let world = core.current_world();
        let masked = world.is_some_and(|w| w.echo_masked);
        let world_type = world.and_then(|w| w.settings.as_ref()).map_or("mud", |s| s.world_type.as_str()).to_string();
        let world_index = core.current;
        let mut cmd = self.editor.text.clone();
        // TF's "^old^new", resolved against this client's history; not at a password
        // prompt, and not in a chat world, where it's just text.
        if !masked && matches!(world_type.as_str(), "mud" | "mud_timed_prompt") {
            match crate::native_gui::core::input::history_substitute(&cmd, &self.editor.history) {
                Some(Err(())) => {
                    self.editor.clear();
                    self.editor.remember("", false);
                    let fx = self.session.core.client_line("% No match.");
                    let rest = self.session.apply(fx);
                    self.after(rest, cx);
                    return;
                }
                Some(Ok(line)) => {
                    cmd = line;
                    let fx = self.session.core.client_line(&cmd);
                    let rest = self.session.apply(fx);
                    self.after(rest, cx);
                }
                None => {}
            }
        }
        let trimmed = cmd.trim();
        let first = trimmed.split_whitespace().next().unwrap_or("");
        // What this client does itself rather than send.
        let local = match first {
            // A remote GUI's /reload restarts the GUI, not the server.
            "/reload" => Some(None),
            "/window" if trimmed.contains("--grep") => Some(Some("% Grep windows aren't in the native GUI yet.")),
            "/server" => Some(Some("% /server isn't in the native GUI yet.")),
            "/import" => Some(Some("% /import isn't in the native GUI yet.")),
            _ => None,
        };
        if let Some(note) = local {
            self.editor.clear();
            let fx = match note {
                None => self.session.core.restart(),
                Some(text) => self.session.core.client_line(text),
            };
            let rest = self.session.apply(fx);
            self.after(rest, cx);
            self.input_touched(cx);
            return;
        }
        let fx = self.session.send(crate::websocket::WsMessage::SendCommand { world_index, command: cmd.clone() });
        self.after(fx, cx);
        self.editor.remember(&cmd, masked);
        self.editor.clear();
        let fx = self.session.core.command_sent();
        let rest = self.session.apply(fx);
        self.after(rest, cx);
        self.input_touched(cx);
    }

    /// A command the server parsed that opens something here (app.js
    /// `executeLocalCommand`).
    pub(super) fn local_command(&mut self, command: &str, cx: &mut Context<Self>) {
        let first = command.split_whitespace().next().unwrap_or("").to_lowercase();
        match first.as_str() {
            "/quit" => cx.quit(),
            "/reload" => {
                let fx = self.session.core.restart();
                let rest = self.session.apply(fx);
                self.after(rest, cx);
            }
            "/menu" => self.later(|v, window, cx| v.open_surface("menu", window, cx), cx),
            "/help" => self.later(|v, window, cx| v.open_surface("help", window, cx), cx),
            "/worlds" | "/world" => {
                let args: Vec<&str> = command.split_whitespace().skip(1).collect();
                self.worlds_command(&args, cx);
            }
            "/actions" => {
                let world = command.split_whitespace().nth(1).map(str::to_string);
                self.open_actions(world, cx);
            }
            "/connections" | "/l" => {
                let fx = self.session.send(crate::websocket::WsMessage::RequestConnectionsList);
                self.after(fx, cx);
            }
            _ => {
                let fx = self.session.core.client_line(&format!("% {first} opens a window the native GUI doesn't have yet."));
                let rest = self.session.apply(fx);
                self.after(rest, cx);
            }
        }
    }
}

impl EntityInputHandler for ClayView {
    fn text_for_range(&mut self, range: Range<usize>, adjusted: &mut Option<Range<usize>>, _w: &mut Window, _cx: &mut Context<Self>) -> Option<String> {
        let e = &self.editor;
        let r = e.from_utf16(range.start)..e.from_utf16(range.end);
        adjusted.replace(e.to_utf16(r.start)..e.to_utf16(r.end));
        Some(e.text[r].to_string())
    }

    fn selected_text_range(&mut self, _ignore: bool, _w: &mut Window, _cx: &mut Context<Self>) -> Option<UTF16Selection> {
        let e = &self.editor;
        let (r, reversed) = match (e.selection(), e.anchor) {
            (Some(r), Some(a)) => (r, a > e.cursor),
            _ => (e.cursor..e.cursor, false),
        };
        Some(UTF16Selection { range: e.to_utf16(r.start)..e.to_utf16(r.end), reversed })
    }

    fn marked_text_range(&self, _w: &mut Window, _cx: &mut Context<Self>) -> Option<Range<usize>> {
        let e = &self.editor;
        e.marked.as_ref().map(|m| e.to_utf16(m.start)..e.to_utf16(m.end))
    }

    fn unmark_text(&mut self, _w: &mut Window, _cx: &mut Context<Self>) {
        self.editor.marked = None;
    }

    fn replace_text_in_range(&mut self, range: Option<Range<usize>>, text: &str, _w: &mut Window, cx: &mut Context<Self>) {
        let e = &mut self.editor;
        let r = range.map(|r| e.from_utf16(r.start)..e.from_utf16(r.end)).or(e.marked.clone()).or(e.selection()).unwrap_or(e.cursor..e.cursor);
        e.set_cursor(r.start, false);
        e.set_cursor(r.end, true);
        e.marked = None;
        e.insert(text);
        self.typed(cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let e = &mut self.editor;
        let r = range.map(|r| e.from_utf16(r.start)..e.from_utf16(r.end)).or(e.marked.clone()).or(e.selection()).unwrap_or(e.cursor..e.cursor);
        e.text.replace_range(r.clone(), text);
        e.marked = (!text.is_empty()).then(|| r.start..r.start + text.len());
        e.anchor = None;
        // Where the input method puts the cursor within what it's composing.
        let within = selected.map(|s| {
            let utf16: Vec<u16> = text.encode_utf16().collect();
            String::from_utf16_lossy(&utf16[..s.end.min(utf16.len())]).len()
        });
        e.cursor = r.start + within.unwrap_or(text.len());
        self.input_touched(cx);
    }

    fn bounds_for_range(&mut self, range: Range<usize>, _element: Bounds<Pixels>, _w: &mut Window, _cx: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        let at = self.editor.from_utf16(range.start);
        let end = self.editor.from_utf16(range.end);
        let l = self.input.layout.borrow();
        let row = &l.rows[l.row_of(at)?];
        let (x0, x1) = (l.x_of(at)?, l.x_of(end).unwrap_or(l.x_of(at)?));
        Some(Bounds::new(point(px(x0), px(row.top)), size(px((x1 - x0).max(1.0)), px(l.row_h))))
    }

    fn character_index_for_point(&mut self, p: Point<Pixels>, _w: &mut Window, _cx: &mut Context<Self>) -> Option<usize> {
        let at = self.input.layout.borrow().hit(f32::from(p.x), f32::from(p.y))?;
        Some(self.editor.to_utf16(at))
    }
}

/// The input line's own state in the view.
pub struct InputState {
    pub layout: InputHandle,
    /// `input_height`: rows shown (1-15).
    pub rows: usize,
    pub activity: Instant,
    caret_was: bool,
    pub dragging: bool,
    /// ^V: the next keystroke goes in as itself.
    pub literal_next: bool,
    /// Byte ranges of misspelled words.
    pub misspelled: Vec<Range<usize>>,
    /// The dictionary, once loaded (in the background: a system word list takes a moment),
    /// and what it was loaded from.
    pub speller: Arc<std::sync::Mutex<Option<Arc<crate::spell::SpellChecker>>>>,
    speller_source: Option<String>,
    /// The system's own checker (Windows, macOS), used unless a dictionary file is set.
    native: Option<super::native_spell::Native>,
    /// The input's right-click menu: where, and the word it's for (with suggestions).
    pub menu: Option<(Point<Pixels>, Option<Suggestions>)>,
}

/// A misspelled word in the input and what it might have been.
pub type Suggestions = (Range<usize>, Vec<String>);

impl Default for InputState {
    fn default() -> Self {
        InputState {
            layout: Rc::new(RefCell::new(InputLayout::default())),
            rows: 3,
            activity: Instant::now(),
            caret_was: true,
            dragging: false,
            literal_next: false,
            misspelled: Vec::new(),
            speller: Arc::new(std::sync::Mutex::new(None)),
            speller_source: None,
            native: None,
            menu: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> InputLayout {
        // "hello world" wrapped after "hello ", 10px a character.
        let row = |top: f32, from: usize, to: usize| InputRow { top, stops: (from..=to).map(|i| (i, (i - from) as f32 * 10.0)).collect() };
        InputLayout { scroll: 0, rows: vec![row(0.0, 0, 6), row(20.0, 6, 11)], row_h: 20.0, bounds: Bounds::default() }
    }

    #[test]
    fn the_caret_moves_between_rows_and_the_mouse_finds_offsets() {
        let l = layout();
        assert_eq!(l.row_of(3), Some(0));
        assert_eq!(l.row_of(6), Some(1), "a wrap point belongs to the row it starts");
        assert_eq!(l.vertical(2, true), Some(8), "down at the same x");
        assert_eq!(l.vertical(8, false), Some(2));
        assert_eq!(l.vertical(2, false), None, "nothing above the first row");
        assert_eq!(l.hit(24.0, 25.0), Some(8));
        assert_eq!(l.hit(500.0, 5.0), Some(6));
    }

    #[test]
    fn a_password_draws_a_dot_per_character_and_controls_as_pictures() {
        let (s, units) = drawn("pa👨\u{200d}👩ss", true);
        assert_eq!(s, "•••••");
        assert_eq!(units.len(), 5);
        let (s, _) = drawn("a\u{1}b\nc", false);
        assert_eq!(s, "a\u{2401}b\nc");
    }
}
