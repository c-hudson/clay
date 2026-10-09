//! F4 Find and F5 Search History, as the web client has them: small popups at the top
//! right. Find shows only the lines containing its text (`*` and `?` are wildcards);
//! Esc-L (`toggle_limit`) closes it and brings it back with the last text. Search cuts
//! the output off at a matching line so it sits at the bottom, Enter stepping to older
//! matches.

use std::borrow::Cow;
use std::sync::OnceLock;

use gpui::{
    AnyElement, Anchor, AppContext, Context, Entity, FontWeight, IntoElement, KeyDownEvent, ParentElement, Styled, Window,
    anchored, deferred, div, point, prelude::*, px, relative,
};
use gpui_component::input::{Input, InputEvent, InputState};
use regex::Regex;

use super::ClayView;
use crate::native_gui::core::client::World;

/// app.js `stripAnsiForFilter`.
pub fn strip_ansi(text: &str) -> Cow<'_, str> {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\x1b\[[0-9;?]*[@-~]").expect("valid")).replace_all(text, "")
}

/// app.js `filterWildcardToRegex`: contains, case-insensitive, `*` any run, `?` one
/// character, `\*` `\?` `\\` literal.
fn wildcard_regex(pattern: &str) -> Option<Regex> {
    let mut re = String::from("(?i)");
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if matches!(chars.peek(), Some('*' | '?' | '\\')) => {
                re.push('\\');
                re.push(chars.next().expect("peeked"));
            }
            '*' => re.push_str(".*"),
            '?' => re.push('.'),
            c => re.push_str(&regex::escape(&c.to_string())),
        }
    }
    Regex::new(&re).ok()
}

/// app.js `matchesFilter`: a plain substring test unless the pattern has a wildcard.
pub fn matches_filter(text: &str, pattern: &str) -> bool {
    if pattern.contains(['*', '?']) {
        wildcard_regex(pattern).is_some_and(|re| re.is_match(text))
    } else {
        text.to_lowercase().contains(&pattern.to_lowercase())
    }
}

/// A line's text as Find and Search see it: no escape codes, no line breaks.
pub fn plain(text: &str) -> String {
    strip_ansi(text).replace(['\r', '\n'], "")
}

pub struct FindState {
    pub filter_open: bool,
    pub filter_text: String,
    /// The last text Find had, for Esc-L.
    last_filter: String,
    pub search_open: bool,
    search_text: String,
    /// The ids of the lines matching the search, oldest first, and the one shown.
    matches: Vec<u64>,
    pos: Option<usize>,
    filter_input: Entity<InputState>,
    search_input: Entity<InputState>,
}

impl FindState {
    pub fn new(window: &mut Window, cx: &mut Context<ClayView>) -> Self {
        let filter_input = cx.new(|cx| InputState::new(window, cx));
        let search_input = cx.new(|cx| InputState::new(window, cx));
        cx.subscribe_in(&filter_input, window, |this, input, ev: &InputEvent, _w, cx| {
            if let InputEvent::Change = ev {
                this.find.filter_text = input.read(cx).value().to_string();
                if !this.find.filter_text.is_empty() {
                    this.find.last_filter = this.find.filter_text.clone();
                }
                this.scroll.borrow_mut().follow_bottom();
                cx.notify();
            }
        })
        .detach();
        cx.subscribe_in(&search_input, window, |this, input, ev: &InputEvent, _w, cx| {
            match ev {
                InputEvent::Change => {
                    this.find.search_text = input.read(cx).value().to_string();
                    this.compute_search();
                }
                InputEvent::PressEnter { .. } => this.advance_search(),
                _ => return,
            }
            this.scroll.borrow_mut().follow_bottom();
            cx.notify();
        })
        .detach();
        FindState {
            filter_open: false,
            filter_text: String::new(),
            last_filter: String::new(),
            search_open: false,
            search_text: String::new(),
            matches: Vec::new(),
            pos: None,
            filter_input,
            search_input,
        }
    }

    /// Find is filtering (open, with text).
    pub fn filtering(&self) -> Option<&str> {
        (self.filter_open && !self.filter_text.is_empty()).then_some(self.filter_text.as_str())
    }

    /// The line the search shows at the bottom, if any.
    pub fn search_cut(&self) -> Option<u64> {
        if !self.search_open {
            return None;
        }
        self.pos.and_then(|p| self.matches.get(p).copied())
    }

    /// "(n/m)" or "(no matches)" beside the search field.
    fn match_info(&self) -> String {
        if self.search_text.is_empty() {
            String::new()
        } else if self.matches.is_empty() {
            "(no matches)".into()
        } else {
            format!("({}/{})", self.pos.map_or(0, |p| p + 1), self.matches.len())
        }
    }
}

/// The ids of a world's lines matching a search, oldest first (gagged ones included, as
/// app.js counts them).
pub fn search_matches(world: &World, text: &str) -> Vec<u64> {
    if text.is_empty() {
        return Vec::new();
    }
    world.lines.iter().filter(|l| matches_filter(&plain(&l.text), text)).map(|l| l.id).collect()
}

impl ClayView {
    fn compute_search(&mut self) {
        let matches = self.session.core.current_world().map(|w| search_matches(w, &self.find.search_text)).unwrap_or_default();
        self.find.pos = matches.len().checked_sub(1);
        self.find.matches = matches;
    }

    /// Enter: the next older match, wrapping to the newest.
    fn advance_search(&mut self) {
        let n = self.find.matches.len();
        if n == 0 {
            return;
        }
        self.find.pos = Some(match self.find.pos {
            Some(p) if p > 0 => p - 1,
            _ => n - 1,
        });
    }

    pub(super) fn toggle_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.find.filter_open {
            self.close_filter(window, cx);
        } else {
            self.find.filter_open = true;
            self.find.filter_text.clear();
            let input = self.find.filter_input.clone();
            input.update(cx, |s, cx| {
                s.set_value("", window, cx);
                s.focus(window, cx);
            });
        }
        cx.notify();
    }

    fn close_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find.filter_open = false;
        self.find.filter_text.clear();
        self.scroll.borrow_mut().follow_bottom();
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Esc-L: close Find, or bring it back with its last text.
    pub(super) fn toggle_limit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.find.filter_open {
            if !self.find.filter_text.is_empty() {
                self.find.last_filter = self.find.filter_text.clone();
            }
            self.close_filter(window, cx);
        } else if !self.find.last_filter.is_empty() {
            self.find.filter_open = true;
            self.find.filter_text = self.find.last_filter.clone();
            let (input, text) = (self.find.filter_input.clone(), self.find.last_filter.clone());
            input.update(cx, |s, cx| s.set_value(text, window, cx));
            self.scroll.borrow_mut().follow_bottom();
            cx.notify();
        }
    }

    pub(super) fn toggle_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.find.search_open {
            self.close_search(window, cx);
        } else {
            self.find.search_open = true;
            self.find.search_text.clear();
            self.find.matches.clear();
            self.find.pos = None;
            let input = self.find.search_input.clone();
            input.update(cx, |s, cx| {
                s.set_value("", window, cx);
                s.focus(window, cx);
            });
        }
        cx.notify();
    }

    fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find.search_open = false;
        self.find.search_text.clear();
        self.find.matches.clear();
        self.find.pos = None;
        self.scroll.borrow_mut().follow_bottom();
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// The snapshot tool's typing: into whichever of the two fields has the focus, then
    /// what a real edit does.
    #[cfg(feature = "native-gui-snapshot")]
    pub(super) fn type_into_find(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let search = gpui::Focusable::focus_handle(self.find.search_input.read(cx), cx).is_focused(window);
        let input = if search { self.find.search_input.clone() } else { self.find.filter_input.clone() };
        input.update(cx, |s, cx| s.insert(text.to_string(), window, cx));
        let value = input.read(cx).value().to_string();
        if search {
            self.find.search_text = value;
            self.compute_search();
        } else {
            self.find.filter_text = value.clone();
            self.find.last_filter = value;
        }
        self.scroll.borrow_mut().follow_bottom();
        cx.notify();
    }

    /// One popup (`.filter-popup`): a header with its title and ✕, then the field.
    fn find_popup(&self, search: bool, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let t = &self.chrome.tokens;
        let (title, label, input) = if search {
            ("Search History [Enter=older]", "Search: ", &self.find.search_input)
        } else {
            ("Find", "Filter: ", &self.find.filter_input)
        };
        let mono = self.fonts.primary.clone();
        let close = div()
            .id(if search { "search-close" } else { "filter-close" })
            .px(px(4.0))
            .cursor_pointer()
            .text_color(t.accent)
            .child("✕")
            .on_click(cx.listener(move |this, _, w, cx| if search { this.close_search(w, cx) } else { this.close_filter(w, cx) }));
        let mut content = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .p(px(8.0))
            .child(div().text_color(t.fg).whitespace_nowrap().child(label))
            .child(
                Input::new(input)
                    .flex_1()
                    // A browser's text input is 20 characters wide by default.
                    .min_w(px(20.0 * 0.6 * self.font_size + 18.0))
                    .text_color(t.fg)
                    .font_family(mono.clone())
                    .text_size(px(self.font_size))
                    .bg(super::output::hsla([self.theme.bg.r, self.theme.bg.g, self.theme.bg.b]))
                    .rounded(px(0.0)),
            );
        if search {
            content = content.child(div().ml(px(6.0)).opacity(0.6).text_color(t.fg).whitespace_nowrap().child(self.find.match_info()));
        }
        div()
            .id(if search { "search-popup" } else { "filter-popup" })
            .occlude()
            .min_w(px(250.0))
            .bg(t.elevated)
            .border_1()
            .border_color(t.accent)
            .rounded(px(4.0))
            .font_family(mono)
            .line_height(relative(1.2))
            .text_size(px(self.font_size))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(8.0))
                    .px(px(8.0))
                    .py(px(4.0))
                    .bg(t.hover)
                    .text_color(t.accent)
                    .text_size(px(12.0))
                    .font_weight(FontWeight::NORMAL)
                    .border_b_1()
                    .border_color(t.accent)
                    .child(title)
                    .child(close),
            )
            .child(content)
            // Escape, and the key that opened it, close it.
            .capture_key_down(cx.listener(move |this, ev: &KeyDownEvent, w, cx| {
                let key = ev.keystroke.key.as_str();
                if key == "escape" || key == if search { "f5" } else { "f4" } {
                    if search { this.close_search(w, cx) } else { this.close_filter(w, cx) }
                    cx.stop_propagation();
                }
            }))
    }

    /// The open popups, top right of the window.
    pub(super) fn find_popups(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.find.filter_open && !self.find.search_open {
            return None;
        }
        let vp = window.viewport_size();
        let mut col = div().flex().flex_col().gap(px(10.0));
        if self.find.filter_open {
            col = col.child(self.find_popup(false, cx));
        }
        if self.find.search_open {
            col = col.child(self.find_popup(true, cx));
        }
        Some(deferred(anchored().anchor(Anchor::TopRight).position(point(vp.width - px(10.0), px(10.0))).child(col)).with_priority(1).into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_matches_like_the_web_client() {
        assert!(matches_filter("You are in a Forest", "forest"));
        assert!(!matches_filter("You are in a Forest", "desert"));
        assert!(matches_filter("hp: 50/100", "hp*/100"));
        assert!(matches_filter("abc", "a?c"));
        assert!(!matches_filter("abc", "a?d"));
        assert!(matches_filter("2*3=6", r"2\*3"));
        assert!(!matches_filter("2x3=6", r"2\*3"));
        assert!(matches_filter("a.b", "a.b*"), "regex characters are literal");
        assert!(!matches_filter("axb", "a.b*"));
        assert_eq!(plain("\x1b[1;31mred\x1b[0m\r\n"), "red");
    }
}
