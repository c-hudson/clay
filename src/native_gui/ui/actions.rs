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
                // Command completion takes Tab first once there is an input area.
                if self.pending_total() > 0 {
                    self.release_screenful(cx);
                } else {
                    self.user_scroll(page);
                }
            }
            "flush_output" => {
                self.release_all(cx);
                self.scroll.borrow_mut().to_bottom();
            }
            "selective_flush" => {
                let fx = self.session.send(WsMessage::SelectiveFlush { world_index: world });
                self.after(fx, cx);
            }
            "pause_output" => self.more.pause(),
            "clear_screen" | "refresh_line" => self.scroll.borrow_mut().to_bottom(),
            "redraw" | "redraw_server_only" => {
                let fx = self.session.core.redraw();
                let rest = self.session.apply(fx);
                self.scroll.borrow_mut().to_bottom();
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
            "world_socket_next" | "world_socket_prev" => {
                let n = self.take_kbnum();
                let n = if action == "world_socket_next" { n } else { -n };
                let fx = self.session.core.cycle_connected(n, now);
                let rest = self.session.apply(fx);
                self.after(rest, cx);
            }
            "bg_all_worlds" => {}
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

    fn world_request(&mut self, which: WorldRequest, cx: &mut Context<Self>) {
        let _ = self.take_kbnum();
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
