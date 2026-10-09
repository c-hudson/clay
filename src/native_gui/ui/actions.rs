//! What a bound key does: the web client's `dispatchActionImpl` for the actions that
//! concern the output and the world list (input editing arrives with the input area).

use std::time::Instant;

use gpui::{Context, Window};

use super::ClayView;
use crate::native_gui::core::client::WorldRequest;
use crate::websocket::WsMessage;

/// TF's `%max_kbnum`.
const MAX_KBNUM: i64 = 999;

impl ClayView {
    /// The numeric prefix's count for an action, unset or zero meaning 1 (app.js
    /// `takeKbnum`); negative reverses.
    fn take_kbnum(&mut self) -> isize {
        let n = self.kbnum.filter(|&n| n != 0).unwrap_or(1);
        self.kbnum = None;
        self.kbnum_sign = 1;
        n as isize
    }

    pub(super) fn run_action(&mut self, action: &str, window: &mut Window, cx: &mut Context<Self>) {
        let now = Instant::now();
        let world = self.session.core.current;
        let view_rows = self.scroll.borrow().view_rows.max(1) as isize;
        let page = (view_rows - 1).max(1);
        match action {
            "scroll_page_up" | "scroll_page_back" => {
                let n = self.take_kbnum();
                self.user_scroll(-page * n);
            }
            "scroll_page_down" => {
                let n = self.take_kbnum();
                if self.scroll.borrow().at_bottom() {
                    if self.pending_total() > 0 {
                        self.release_screenful(cx);
                    } else {
                        self.more.paused = false;
                    }
                } else {
                    self.user_scroll(page * n);
                }
            }
            "scroll_half_page" => {
                let n = self.take_kbnum();
                if self.scroll.borrow().at_bottom() && self.pending_total() > 0 {
                    let rows = ((view_rows - 2) / 2).max(1) as usize * n.max(1) as usize;
                    self.release_lines(rows, cx);
                } else {
                    self.user_scroll((view_rows / 2).max(1) * n);
                }
            }
            "scroll_half_page_back" => {
                let n = self.take_kbnum();
                self.user_scroll(-(view_rows / 2).max(1) * n);
            }
            "scroll_line_forward" => {
                let n = self.take_kbnum();
                self.user_scroll(n);
            }
            "scroll_line_back" => {
                let n = self.take_kbnum();
                self.user_scroll(-n);
            }
            "tab_key" => {
                // Completing a /command comes first.
                if self.complete_input() {
                    self.input_touched(cx);
                } else if self.pending_total() > 0 {
                    self.release_screenful(cx);
                } else {
                    self.user_scroll(page);
                }
            }
            "flush_output" => {
                self.release_all(cx);
                self.scroll.borrow_mut().follow_bottom();
            }
            "selective_flush" => {
                let fx = self.session.send(WsMessage::SelectiveFlush { world_index: world });
                self.after(fx, cx);
            }
            "pause_output" => self.more.pause(),
            "filter_popup" => self.toggle_filter(window, cx),
            "search_popup" => self.toggle_search(window, cx),
            "toggle_limit" => self.toggle_limit(window, cx),
            // TF CLEAR (Esc-^L): "clear screen (can be refilled with scrollback)" - the
            // %textdiv=clear blank after the newest line, until the world is left.
            "clear_screen" => {
                // The world on screen counts as arrived at first, or a late arrival would
                // take this visit's blank away again.
                self.follow_world();
                let after = self.session.core.worlds.get(self.session.core.current).and_then(super::textdiv::last_seq);
                self.textdiv.visit = Some((after, "clear".into()));
                self.scroll.borrow_mut().follow_bottom();
            }
            "refresh_line" => self.scroll.borrow_mut().follow_bottom(),
            "redraw" | "redraw_server_only" => {
                let fx = self.session.core.redraw();
                let rest = self.session.apply(fx);
                self.scroll.borrow_mut().follow_bottom();
                self.after(rest, cx);
            }
            "toggle_tags" => {
                let fx = self.session.send(WsMessage::SendCommand { world_index: world, command: "/tag".into() });
                self.after(fx, cx);
            }
            "toggle_action_highlight" => {
                self.action_highlight = !self.action_highlight;
                self.restyle();
            }
            "world_next" | "world_all_next" | "world_forward" => self.world_request(WorldRequest::Next, cx),
            "world_prev" | "world_all_prev" | "world_previous" => self.world_request(WorldRequest::Prev, cx),
            "world_activity" => self.world_request(WorldRequest::OldestPending, cx),
            "world_socket_next" | "world_socket_prev" if self.mode.lock.is_none() => {
                let n = self.take_kbnum();
                let n = if action == "world_socket_next" { n } else { -n };
                let fx = self.session.core.cycle_connected(n, now);
                let rest = self.session.apply(fx);
                self.after(rest, cx);
            }
            "bg_all_worlds" | "expand_line" | "suspend" | "bell" => {}
            "completion" => {
                self.complete_input();
                self.input_touched(cx);
            }
            "show_version" => {
                let fx = self.session.send(WsMessage::SendCommand { world_index: world, command: "/version".into() });
                self.after(fx, cx);
            }
            "toggle_gmcp_media" => {
                let fx = self.session.send(WsMessage::ToggleWorldGmcp { world_index: world });
                self.after(fx, cx);
            }
            "input_grow" => self.input.rows = (self.input.rows + 1).min(15),
            "input_shrink" => self.input.rows = self.input.rows.saturating_sub(1).max(1),
            "reload" => self.relaunch(None, cx),
            "help" | "emoji_picker" | "spell_check" | "recent_worlds" => self.open_surface(action, window, cx),
            a if self.edit_action(a, cx) => {}
            "kbnum_negative" => {
                self.kbnum = None;
                self.kbnum_sign = -1;
            }
            a if a.starts_with("kbnum_") => {
                if let Ok(d) = a[6..].parse::<i64>() {
                    let magnitude = self.kbnum.map_or(d, |k| (k.abs() * 10 + d).min(MAX_KBNUM));
                    self.kbnum = Some(self.kbnum_sign * magnitude);
                }
            }
            "quit" => cx.quit(),
            other => {
                crate::debug_log(crate::is_debug_enabled(), &format!("native-gui: action {other} not available yet"));
                return;
            }
        }
        let _ = window;
        cx.notify();
    }

    /// Tab completion over the commands and the actions with no pattern (app.js
    /// `completeCommand`).
    fn complete_input(&mut self) -> bool {
        let mut cmds: Vec<String> = crate::native_gui::core::input::internal_commands().to_vec();
        cmds.extend(self.session.core.actions.iter().filter(|a| a.pattern.trim().is_empty()).map(|a| format!("/{}", a.name)));
        self.editor.complete(&cmds)
    }

    /// The input line's editing actions; false for any other action.
    fn edit_action(&mut self, action: &str, cx: &mut Context<Self>) -> bool {
        use crate::native_gui::core::input::Case;
        let positive = |n: isize| if n > 0 { n as usize } else { 1 };
        match action {
            "cursor_left" => {
                let n = self.take_kbnum() as i64;
                self.editor.cursor_left(n);
            }
            "cursor_right" => {
                let n = self.take_kbnum() as i64;
                self.editor.cursor_right(n);
            }
            "cursor_word_left" => {
                let n = self.take_kbnum() as i64;
                self.editor.word_left(n);
            }
            "cursor_word_right" => {
                let n = self.take_kbnum() as i64;
                self.editor.word_right(n);
            }
            "cursor_home" => self.editor.home(),
            "cursor_end" => self.editor.end(),
            // A row up or down in the text; past the first (last) row, to the start (end),
            // as a text area does.
            "cursor_up" | "cursor_down" => {
                let down = action == "cursor_down";
                let to = self.input.layout.borrow().vertical(self.editor.cursor, down);
                match to {
                    Some(at) => self.editor.set_cursor(at, false),
                    None if down => self.editor.end(),
                    None => self.editor.home(),
                }
            }
            "delete_backward" => {
                let n = self.take_kbnum() as i64;
                self.editor.delete_backward(n);
            }
            "delete_forward" => {
                let n = self.take_kbnum() as i64;
                self.editor.delete_forward(n);
            }
            "delete_word_backward" => {
                let n = self.take_kbnum() as i64;
                self.editor.delete_word_backward(n);
            }
            "delete_word_forward" => {
                let n = self.take_kbnum() as i64;
                self.editor.delete_word_forward(n);
            }
            "delete_word_backward_punct" => {
                let n = self.take_kbnum() as i64;
                self.editor.delete_word_backward_punct(n);
            }
            "kill_to_end" => self.editor.kill_to_end(),
            "kill_to_start" => self.editor.kill_to_start(),
            "clear_line" => self.editor.clear_line(),
            "transpose_chars" => {
                for _ in 0..positive(self.take_kbnum()) {
                    self.editor.transpose();
                }
            }
            "capitalize_word" | "lowercase_word" | "uppercase_word" => {
                let how = match action {
                    "capitalize_word" => Case::Capitalize,
                    "lowercase_word" => Case::Lower,
                    _ => Case::Upper,
                };
                for _ in 0..positive(self.take_kbnum()) {
                    self.editor.transform_word(how);
                }
            }
            "collapse_spaces" => self.editor.collapse_spaces(),
            "scramble_words" => self.editor.scramble(),
            "goto_matching_bracket" => self.editor.goto_matching_bracket(),
            "insert_last_arg" => self.editor.insert_last_arg(),
            "yank" => self.editor.yank(),
            "literal_next" => self.input.literal_next = true,
            "toggle_insert" => self.editor.overwrite = !self.editor.overwrite,
            "history_prev" => {
                let n = self.take_kbnum() as i64;
                self.editor.history_prev(n);
            }
            "history_next" => {
                let n = self.take_kbnum() as i64;
                self.editor.history_next(n);
            }
            "history_search_backward" => {
                let n = self.take_kbnum() as i64;
                self.editor.search_backward_n(n);
            }
            "history_search_forward" => {
                let n = self.take_kbnum() as i64;
                self.editor.search_forward_n(n);
            }
            "recall_begin" => {
                let n = self.take_kbnum() as i64;
                self.editor.recall_begin(n);
            }
            "recall_end" => {
                let n = self.take_kbnum() as i64;
                self.editor.recall_end(n);
            }
            _ => return false,
        }
        self.input_touched(cx);
        true
    }

    fn world_request(&mut self, which: WorldRequest, cx: &mut Context<Self>) {
        let _ = self.take_kbnum();
        if self.mode.lock.is_some() {
            return;
        }
        let fx = self.session.core.request_world(which);
        let rest = self.session.apply(fx);
        self.after(rest, cx);
    }

    /// A scroll the user asked for: moving up leaves the bottom (and pauses output, as
    /// app.js does when more-mode is on).
    pub(super) fn user_scroll(&mut self, rows: isize) {
        if rows < 0 && self.more_mode_enabled() && !self.more.paused {
            self.more.pause();
        }
        self.scroll.borrow_mut().by(rows);
    }

    /// Local held lines plus the server's.
    pub(super) fn pending_total(&self) -> usize {
        self.more.held() + self.session.core.current_world().map_or(0, |w| w.pending_count)
    }

    pub(super) fn release_lines(&mut self, n: usize, cx: &mut Context<Self>) {
        self.more.release(n);
        self.more.caught_up();
        let world = self.session.core.current;
        if self.session.core.current_world().is_some_and(|w| w.pending_count > 0) {
            let fx = self.session.core.release_pending(world, n);
            let rest = self.session.apply(fx);
            self.after(rest, cx);
        }
        cx.notify();
    }

    pub(super) fn release_screenful(&mut self, cx: &mut Context<Self>) {
        let rows = self.scroll.borrow().view_rows.saturating_sub(2).max(1);
        self.release_lines(rows, cx);
    }

    pub(super) fn release_all(&mut self, cx: &mut Context<Self>) {
        self.more.release_all();
        let world = self.session.core.current;
        let fx = self.session.core.release_pending(world, 0);
        let rest = self.session.apply(fx);
        self.after(rest, cx);
    }
}

#[cfg(test)]
mod tests {
    /// Every bindable action has a case here (implemented, a deliberate no-op, or a window
    /// that comes in a later phase), so a key bound to a new action can't silently do
    /// nothing in the native GUI.
    #[test]
    fn every_bindable_action_has_a_case() {
        let src = include_str!("actions.rs");
        let missing: Vec<&str> = crate::keybindings::ACTIONS
            .iter()
            .map(|a| a.id)
            .filter(|id| !id.starts_with("kbnum_") && !src.contains(&format!("\"{id}\"")))
            .collect();
        assert!(missing.is_empty(), "no case for: {missing:?}");
    }
}
