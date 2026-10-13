//! The emoji picker (Esc-e; app.js `openEmojiPopup`), a window of its own: a search box,
//! a tab per category, and a grid. Its filter is the console's `filter_emoji`, which
//! app.js's `getFilteredEmoji` mirrors one for one. Arrow keys move between three zones
//! (search, tabs, grid) as the console's and the web's do; the keyboard stays in the
//! search box the whole time, so typing always searches.
//!
//! A click on a cell puts the emoji in the input of the window that opened the picker
//! and leaves the picker up - the user may want several; the ✕ (or Esc) closes it. Enter
//! inserts the selected one and closes, as before.

use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, KeyBinding, ParentElement, Render,
    ScrollHandle, SharedString, StatefulInteractiveElement, Styled, WeakEntity, Window, div, prelude::*, px, relative,
};
use gpui_component::input::{Input, InputEvent, InputState};

use crate::emoji::{Category, EMOJI};
use crate::native_gui::ui::ClayView;
use crate::native_gui::ui::chrome::alpha;
use crate::native_gui::ui::dialogs::DialogEvent;

gpui::actions!(emoji_picker, [Up, Down, Left, Right, Home, End, PageUp, PageDown, Enter, Escape, Tab]);

const CONTEXT: &str = "EmojiPicker";
/// The window's width (`windows::Kind::Emoji`): the web picker's (style.css
/// `min(92vw, 420px)`). Its height is `height()`.
pub const WIDTH: f32 = 420.0;
/// The grid's cells: tighter than the web's 38px (user ruling: the window no bigger than
/// it has to be), twelve to a row at `WIDTH`.
const CELL_MIN: f32 = 32.0;
const GAP: f32 = 2.0;
const GRID_PAD: f32 = 6.0;
/// What the header, tab strip and search row take above the grid, as laid out in
/// `render` (measured on a capture; a padding change there must be mirrored here).
const CHROME: f32 = 123.0;
/// A page of the grid before it has been drawn.
const GRID_H_FALLBACK: f32 = 208.0;

/// The window's height: the chrome plus a grid tall enough for the largest category
/// (Smileys), so no tab scrolls. A search with more matches than that does.
pub fn height() -> f32 {
    let largest = Category::all().iter().map(|c| EMOJI.iter().filter(|e| e.category == *c).count()).max().unwrap_or(0);
    let rows = largest.div_ceil(columns_for(WIDTH)).max(1) as f32;
    (CHROME + 2.0 * GRID_PAD + rows * CELL_MIN + (rows - 1.0) * GAP + 2.0).ceil()
}

/// The picker's keys, matched while its search box has the keyboard: a deeper context
/// than the text field's own, so arrows mean zones before they mean the caret.
pub fn bind_keys(cx: &mut App) {
    let at = Some("EmojiPicker > Input");
    cx.bind_keys([
        KeyBinding::new("up", Up, at),
        KeyBinding::new("down", Down, at),
        KeyBinding::new("left", Left, at),
        KeyBinding::new("right", Right, at),
        KeyBinding::new("home", Home, at),
        KeyBinding::new("end", End, at),
        KeyBinding::new("pageup", PageUp, at),
        KeyBinding::new("pagedown", PageDown, at),
        KeyBinding::new("enter", Enter, at),
        KeyBinding::new("escape", Escape, at),
        KeyBinding::new("tab", Tab, at),
        KeyBinding::new("shift-tab", Tab, at),
    ]);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Zone {
    Search,
    Tabs,
    Grid,
}

pub struct EmojiPicker {
    /// The view whose input a pick goes into: the window that opened the picker.
    main: WeakEntity<ClayView>,
    search: Entity<InputState>,
    query: String,
    /// The tab: an index into `Category::all()`.
    category: usize,
    selected: usize,
    zone: Zone,
    grid: ScrollHandle,
    /// The grid's column count at the last draw: the window's width decides it.
    columns: usize,
}

/// What a category and a query show (the console's rule; a query searches every
/// category).
fn matches(category: Option<Category>, query: &str) -> Vec<(&'static str, &'static str)> {
    crate::popup::definitions::emoji::filter_emoji(category, query)
        .into_iter()
        .filter_map(|item| EMOJI.iter().find(|e| e.ch == item.id).map(|e| (e.ch, e.name)))
        .collect()
}

/// How many columns the grid fits in a window `width` wide (its own padding inside).
fn columns_for(width: f32) -> usize {
    let inner = width - 2.0 * GRID_PAD;
    (((inner + GAP) / (CELL_MIN + GAP)).floor() as usize).max(1)
}

/// A cell's side for `columns` across `width`: a hair under the exact share, so
/// rounding can't push the last one to a new row.
fn cell_size(width: f32, columns: usize) -> f32 {
    let inner = width - 2.0 * GRID_PAD;
    ((inner - (columns as f32 - 1.0) * GAP) / columns as f32).floor()
}

impl EmojiPicker {
    pub fn new(main: WeakEntity<ClayView>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search emoji..."));
        cx.subscribe_in(&search, window, |this, input, ev: &InputEvent, _window, cx| {
            if let InputEvent::Change = ev {
                this.query = input.read(cx).value().to_string();
                this.selected = 0;
                this.zone = Zone::Search;
                cx.notify();
            }
        })
        .detach();
        Self { main, search, query: String::new(), category: 0, selected: 0, zone: Zone::Search, grid: ScrollHandle::new(), columns: columns_for(WIDTH) }
    }

    /// Asked for again from `main`'s window: picks go there now.
    pub fn retarget(&mut self, main: WeakEntity<ClayView>, cx: &mut Context<Self>) {
        self.main = main;
        cx.notify();
    }

    /// The category a filter scopes to: none while searching (app.js `emojiActiveCategory`).
    fn active(&self) -> Option<Category> {
        if self.query.is_empty() { Category::all().get(self.category).copied() } else { None }
    }

    fn list(&self) -> Vec<(&'static str, &'static str)> {
        matches(self.active(), &self.query)
    }

    /// A deliberate pick of a category: its own emoji, the search cleared (wraps).
    fn set_category(&mut self, index: isize, window: &mut Window, cx: &mut Context<Self>) {
        let n = Category::all().len() as isize;
        self.category = index.rem_euclid(n) as usize;
        self.query.clear();
        self.search.update(cx, |s, cx| s.set_value("", window, cx));
        self.selected = 0;
        self.grid.scroll_to_item(0);
        cx.notify();
    }

    /// app.js `emojiGridMove` (the console's `grid_move`): sideways runs across rows and
    /// stops at the ends; up and down keep the column, clamped on a short last row.
    fn grid_move(&mut self, dx: isize, dy: isize) {
        let len = self.list().len();
        if len == 0 {
            return;
        }
        self.selected = grid_step(self.selected, len, self.columns, dx, dy);
        self.grid.scroll_to_item(self.selected);
    }

    /// The rows the grid shows at once, from its last layout.
    fn page_rows(&self) -> isize {
        let h = f32::from(self.grid.bounds().size.height);
        let h = if h > 0.0 { h } else { GRID_H_FALLBACK };
        ((h - 2.0 * GRID_PAD + GAP) / (CELL_MIN + GAP)).floor().max(1.0) as isize
    }

    fn grid_page(&mut self, delta: isize) {
        self.grid_move(0, delta * self.page_rows());
    }

    /// Put cell `index` in the opener's input; `close` the window too (Enter), or keep
    /// it for the next pick (a click).
    fn insert(&mut self, index: usize, close: bool, cx: &mut Context<Self>) {
        let Some((ch, _)) = self.list().get(index).copied() else { return };
        let _ = self.main.update(cx, |v, cx| v.insert_text(ch, cx));
        if close {
            cx.emit(DialogEvent::Close);
        }
    }

    fn key(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let len = self.list().len();
        match (key, self.zone) {
            // The search box's own caret keys.
            ("left" | "right" | "home" | "end", Zone::Search) => return cx.propagate(),
            ("tab", _) => {}
            ("escape", _) => cx.emit(DialogEvent::Close),
            ("enter", _) => self.insert(self.selected, true, cx),
            ("pagedown", _) => self.grid_page(1),
            ("pageup", _) => self.grid_page(-1),
            ("up", Zone::Tabs) => self.zone = Zone::Search,
            ("up", Zone::Grid) => {
                if self.selected < self.columns {
                    self.zone = Zone::Tabs;
                } else {
                    self.grid_move(0, -1);
                }
            }
            ("down", Zone::Search) => self.zone = Zone::Tabs,
            ("down", Zone::Tabs) => self.zone = Zone::Grid,
            ("down", Zone::Grid) => self.grid_move(0, 1),
            ("left", Zone::Tabs) => self.set_category(self.category as isize - 1, window, cx),
            ("right", Zone::Tabs) => self.set_category(self.category as isize + 1, window, cx),
            ("home", Zone::Tabs) => self.set_category(0, window, cx),
            ("end", Zone::Tabs) => self.set_category(-1, window, cx),
            ("left", Zone::Grid) => self.grid_move(-1, 0),
            ("right", Zone::Grid) => self.grid_move(1, 0),
            ("home", Zone::Grid) => {
                self.selected = 0;
                self.grid.scroll_to_item(0);
            }
            ("end", Zone::Grid) => {
                self.selected = len.saturating_sub(1);
                self.grid.scroll_to_item(self.selected);
            }
            _ => {}
        }
        cx.notify();
    }
}

/// One move on a `cols`-wide grid of `len` cells from `at` (`PopupState::grid_move`).
fn grid_step(at: usize, len: usize, cols: usize, dx: isize, dy: isize) -> usize {
    let cols = cols.max(1);
    let mut at = at.min(len.saturating_sub(1));
    if dx != 0 {
        let n = at as isize + dx;
        if n >= 0 && (n as usize) < len {
            at = n as usize;
        }
    }
    if dy != 0 {
        let rows = len.div_ceil(cols) as isize;
        let (row, col) = ((at / cols) as isize, at % cols);
        let new_row = (row + dy).clamp(0, rows - 1) as usize;
        let row_len = (len - new_row * cols).min(cols);
        at = new_row * cols + col.min(row_len.saturating_sub(1));
    }
    at
}

impl EventEmitter<DialogEvent> for EmojiPicker {}
impl Focusable for EmojiPicker {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.search.read(cx).focus_handle(cx)
    }
}

impl Render for EmojiPicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(k) = self.main.upgrade().map(|m| m.read(cx).kit()) else { return div().into_any_element() };
        let selection_bg = self.main.upgrade().map(|m| m.read(cx).selection_color()).unwrap_or(k.t.hover);
        let t = k.t.clone();
        let width = f32::from(window.viewport_size().width);
        self.columns = columns_for(width);
        let list = self.list();
        if self.selected >= list.len() {
            self.selected = list.len().saturating_sub(1);
        }
        let searching = !self.query.is_empty();
        let title = match self.active() {
            Some(c) => format!("Emoji — {}", c.label()),
            None => "Emoji".to_string(),
        };
        let tabs = Category::all().iter().enumerate().map(|(i, c)| {
            let active = !searching && i == self.category;
            let filled = active && self.zone == Zone::Tabs;
            let (fg, hover_fg) = (t.muted, t.fg);
            let tab = div()
                .id(("emoji-tab", i))
                .px(px(10.0))
                .pt(px(6.0))
                .pb(px(7.0))
                .rounded_t(px(5.0))
                .border_b_2()
                .font_family(k.fonts.emoji.clone())
                .text_size(px(18.0))
                .line_height(relative(1.0))
                .cursor_pointer()
                .child(c.tab_glyph());
            let tab = if filled {
                tab.bg(t.accent).border_color(t.accent).text_color(gpui::black())
            } else if active {
                tab.bg(gpui::Hsla { a: 0.12, ..t.accent }).border_color(t.accent).text_color(t.accent)
            } else {
                tab.border_color(gpui::transparent_black()).text_color(fg).hover(move |s| s.bg(alpha([255, 255, 255], 0.05)).text_color(hover_fg))
            };
            tab.on_click(cx.listener(move |this, _, window, cx| {
                this.set_category(i as isize, window, cx);
                this.zone = Zone::Tabs;
            }))
        });
        let cell = cell_size(width, self.columns);
        let grid_focused = self.zone == Zone::Grid;
        let cells = list.iter().enumerate().map(|(i, (ch, name))| {
            let selected = i == self.selected;
            let c = div()
                .id(("emoji-cell", i))
                .size(px(cell))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .font_family(k.fonts.emoji.clone())
                .text_size(px(21.0))
                .line_height(relative(1.0))
                .cursor_pointer()
                .child(*ch)
                .tooltip({
                    let tip = SharedString::from(format!(":{name}:"));
                    move |w, cx| gpui_component::tooltip::Tooltip::new(tip.clone()).build(w, cx)
                });
            let c = match (selected, grid_focused) {
                (true, true) => c.bg(t.accent).text_color(gpui::black()),
                (true, false) => c.bg(selection_bg),
                _ => c.hover(|s| s.bg(alpha([255, 255, 255], 0.08))),
            };
            // A click: into the input, and the picker stays for the next one.
            c.on_click(cx.listener(move |this, _, _, cx| {
                this.selected = i;
                this.insert(i, false, cx);
                cx.notify();
            }))
        });
        let empty = list.is_empty().then(|| {
            div().w_full().py(px(24.0)).px(px(8.0)).text_center().text_size(px(13.0)).text_color(t.muted).child(format!("No emoji match “{}”", self.query))
        });
        let search_border = if self.zone == Zone::Search { t.accent } else { t.border };
        div()
            .id("emoji-window")
            .key_context(CONTEXT)
            .size_full()
            .flex()
            .flex_col()
            .font_family(k.fonts.ui.clone())
            .text_size(px(13.0))
            .text_color(t.fg)
            .on_action(cx.listener(|this, _: &Up, w, cx| this.key("up", w, cx)))
            .on_action(cx.listener(|this, _: &Down, w, cx| this.key("down", w, cx)))
            .on_action(cx.listener(|this, _: &Left, w, cx| this.key("left", w, cx)))
            .on_action(cx.listener(|this, _: &Right, w, cx| this.key("right", w, cx)))
            .on_action(cx.listener(|this, _: &Home, w, cx| this.key("home", w, cx)))
            .on_action(cx.listener(|this, _: &End, w, cx| this.key("end", w, cx)))
            .on_action(cx.listener(|this, _: &PageUp, w, cx| this.key("pageup", w, cx)))
            .on_action(cx.listener(|this, _: &PageDown, w, cx| this.key("pagedown", w, cx)))
            .on_action(cx.listener(|this, _: &Enter, w, cx| this.key("enter", w, cx)))
            .on_action(cx.listener(|this, _: &Escape, w, cx| this.key("escape", w, cx)))
            .on_action(cx.listener(|this, _: &Tab, w, cx| this.key("tab", w, cx)))
            .child(div().px(px(14.0)).py(px(8.0)).border_b_1().border_color(t.border_subtle).child(k.header(
                "emoji-close",
                title,
                cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close)),
            )))
            .child(div().flex().gap(px(2.0)).px(px(8.0)).pt(px(6.0)).children(tabs))
            .child(
                div().px(px(10.0)).pt(px(6.0)).pb(px(8.0)).border_b_1().border_color(t.border_subtle).child(
                    Input::new(&self.search)
                        .w_full()
                        .bg(k.field_bg)
                        .text_color(t.fg)
                        .font_family(k.fonts.primary.clone())
                        .text_size(px(13.0))
                        .rounded(px(6.0))
                        // The keyboard never leaves this box, so its own focus ring would
                        // always be lit; the accent marks the Search zone instead.
                        .focus_bordered(false)
                        .border_color(search_border),
                ),
            )
            .child(
                div()
                    .id("emoji-grid")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.grid)
                    .p(px(GRID_PAD))
                    .flex()
                    .flex_wrap()
                    .content_start()
                    .gap(px(GAP))
                    .children(cells)
                    .children(empty),
            )
            .font_weight(FontWeight::NORMAL)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_moves_are_the_consoles() {
        // 23 cells, 10 to a row: rows of 10, 10, 3.
        assert_eq!(grid_step(9, 23, 10, 1, 0), 10, "right off a row's end: the next row");
        assert_eq!(grid_step(22, 23, 10, 1, 0), 22, "the last cell stays");
        assert_eq!(grid_step(0, 23, 10, -1, 0), 0);
        assert_eq!(grid_step(17, 23, 10, 0, 1), 22, "down into the short row: its last cell");
        assert_eq!(grid_step(5, 23, 10, 0, -1), 5, "up from the top row: stays");
        assert_eq!(grid_step(5, 23, 10, 0, 50), 22);
    }

    #[test]
    fn a_query_searches_every_category_and_tabs_scope_without_one() {
        let smileys = matches(Some(Category::Smileys), "");
        assert!(!smileys.is_empty());
        let everywhere = matches(None, "cat");
        assert!(everywhere.iter().any(|(_, n)| n.contains("cat")));
    }

    #[test]
    fn the_grid_fits_the_window() {
        assert_eq!(columns_for(WIDTH), 12, "twelve 32px cells in 420px");
        assert_eq!(columns_for(600.0), 17);
        assert_eq!(columns_for(10.0), 1, "never zero columns");
        let cell = cell_size(WIDTH, 12);
        assert!(cell >= CELL_MIN && 12.0 * cell + 11.0 * GAP <= WIDTH - 2.0 * GRID_PAD, "cell {cell}");
    }

    #[test]
    fn the_window_is_as_tall_as_the_largest_category_needs() {
        let smileys = EMOJI.iter().filter(|e| e.category == Category::Smileys).count();
        let largest = Category::all().iter().map(|c| EMOJI.iter().filter(|e| e.category == *c).count()).max().unwrap();
        assert_eq!(largest, smileys, "Smileys is the largest category; `height()` follows whichever is");
        let rows = smileys.div_ceil(12);
        assert_eq!(rows, 7, "{smileys} smileys, twelve to a row");
        let h = height();
        assert!(h > CHROME + 7.0 * CELL_MIN && h < CHROME + 8.0 * (CELL_MIN + GAP) + 2.0 * GRID_PAD, "{h}");
    }
}
