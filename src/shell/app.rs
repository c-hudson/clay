//! The App side of shell worlds: connecting, reading the screen, activity, input, frames.
//!
//! These are `App` methods, kept here rather than in main.rs because they are their own
//! concern and touch nothing a MUD line touches. A shell world's text never enters
//! `output_lines`, the log, the archive, TF or the triggers - it lives entirely in the
//! emulator (`ShellSession`) and the screen cache (`World::shell_screen`).

use std::time::Instant;

use crate::shell::{self, screen, ShellEvent, ShellScreen, ShellSession, Size};
use crate::websocket::{ShellKey, WsMessage};
use crate::{tf, AppEvent};

impl crate::App {
    /// True if world `idx` is a live shell world: a running session on the server, or (on
    /// a client that has no session of its own, like the SSH remote console) a screen the
    /// server still reports as live.
    pub(crate) fn shell_live(&self, idx: usize) -> bool {
        self.worlds.get(idx).is_some_and(|w| {
            w.shell.as_ref().is_some_and(|s| s.is_live()) || (w.shell.is_none() && w.shell_screen.as_ref().is_some_and(|s| s.live))
        })
    }

    /// True if the console is currently showing a live shell world.
    pub(crate) fn shell_live_current(&self) -> bool {
        self.shell_live(self.current_world_index)
    }

    /// Start a shell for world `idx`. Reports back as `AppEvent::Shell`.
    pub(crate) fn spawn_shell_connect(&mut self, idx: usize) {
        let Some(events) = self.event_tx.clone() else { return };
        // Drop any previous session (its late events go stale on the bump).
        self.worlds[idx].shell = None;
        self.worlds[idx].connection_id += 1;
        let conn_id = self.worlds[idx].connection_id;
        let name = self.worlds[idx].name.clone();
        let command = self.worlds[idx].settings.hostname.trim();
        let command = (!command.is_empty()).then(|| command.to_string());
        let size = self.shell_dimensions(idx);
        match ShellSession::start(&name, conn_id, command.as_deref(), size, events) {
            Ok(session) => {
                self.worlds[idx].shell = Some(session);
                self.worlds[idx].shell_screen = Some(ShellScreen::new(size.cols, size.rows));
                self.worlds[idx].connected = true;
                self.worlds[idx].was_connected = true;
                self.worlds[idx].reconnect_at = None;
                self.ws_broadcast(WsMessage::WorldConnected { world_index: idx, name });
                if idx == self.current_world_index {
                    self.needs_output_redraw = true;
                }
            }
            Err(e) => {
                self.add_output_to_world(idx, &format!("Cannot start shell: {e}"));
            }
        }
    }

    /// A shell session reported something (`AppEvent::Shell`). Always fully handled.
    pub(crate) fn handle_shell_event(&mut self, world_name: String, conn_id: u64, ev: ShellEvent) -> Option<AppEvent> {
        let idx = self.find_world_index(&world_name)?;
        if self.worlds[idx].connection_id != conn_id {
            return None; // a session already replaced or shut down
        }
        match ev {
            ShellEvent::Wakeup => self.refresh_shell_screen(idx),
            ShellEvent::Title(title) => {
                if let Some(screen) = self.worlds[idx].shell_screen.as_mut() {
                    screen.set_title(title);
                }
                if idx == self.current_world_index {
                    self.needs_output_redraw = true;
                }
            }
            ShellEvent::Bell => {
                // The terminal's own bell; nothing beyond what the screen frame carries.
            }
            ShellEvent::Exited(code) => self.handle_shell_exited(idx, code),
        }
        None
    }

    /// Read the emulator into the screen cache; count activity; mark for redraw.
    pub(crate) fn refresh_shell_screen(&mut self, idx: usize) {
        let Some(session) = self.worlds[idx].shell.as_ref() else { return };
        if !session.take_wakeup() {
            return;
        }
        let snapshot = session.with_term(screen::render_rows);
        let is_current = idx == self.current_world_index || self.ws_client_viewing(idx);
        let Some(screen) = self.worlds[idx].shell_screen.as_mut() else { return };
        let applied = screen.apply(snapshot);
        if applied.gained_printable && !is_current {
            let unseen_before = self.worlds[idx].unseen_lines;
            if unseen_before == 0 && self.worlds[idx].first_unseen_at.is_none() {
                self.worlds[idx].first_unseen_at = Some(Instant::now());
            }
            self.worlds[idx].unseen_lines += 1;
            let count = self.worlds[idx].unseen_lines;
            self.ws_broadcast(WsMessage::UnseenUpdate { world_index: idx, count });
            self.broadcast_activity();
            if unseen_before == 0 {
                let name = self.worlds[idx].name.clone();
                self.fire_tf_hook(Some(idx), tf::TfHookEvent::Preactivity, &name, false);
                self.fire_tf_hook(Some(idx), tf::TfHookEvent::Activity, &name, false);
            }
        }
        if applied.changed && idx == self.current_world_index {
            self.needs_output_redraw = true;
        }
    }

    /// The shell exited: a note in the grid, a frozen final frame, the world disconnected.
    pub(crate) fn handle_shell_exited(&mut self, idx: usize, code: Option<i32>) {
        let note = match code {
            Some(0) => "\r\n[Clay: shell exited]\r\n".to_string(),
            Some(c) => format!("\r\n[Clay: shell exited (code {c})]\r\n"),
            None => "\r\n[Clay: shell ended]\r\n".to_string(),
        };
        if let Some(session) = self.worlds[idx].shell.as_ref() {
            session.inject(note.as_bytes());
            let snapshot = session.with_term(screen::render_rows);
            if let Some(screen) = self.worlds[idx].shell_screen.as_mut() {
                screen.apply(snapshot);
            }
        }
        if let Some(screen) = self.worlds[idx].shell_screen.as_mut() {
            screen.set_exited();
        }
        self.worlds[idx].shell = None;
        self.worlds[idx].connected = false;
        self.worlds[idx].reconnect_at = None;
        if idx == self.current_world_index {
            self.needs_output_redraw = true;
        }
        self.flush_one_shell_frame(idx);
        self.ws_broadcast(WsMessage::WorldDisconnected { world_index: idx });
    }

    /// The size a shell world's PTY should be: the smallest over the console (when it is
    /// showing the world) and every client currently viewing it. Nobody looking keeps the
    /// current size.
    pub(crate) fn shell_dimensions(&self, idx: usize) -> Size {
        let mut cols: Option<u16> = None;
        let mut rows: Option<u16> = None;
        let mut take = |w: u16, h: u16| {
            if w > 0 && h > 0 {
                cols = Some(cols.map_or(w, |c| c.min(w)));
                rows = Some(rows.map_or(h, |r| r.min(h)));
            }
        };
        if self.console_tui && self.current_world_index == idx {
            take(self.output_width, self.output_height);
        }
        for v in self.ws_client_worlds.values() {
            if v.world_index != idx || !v.visible || v.paused {
                continue;
            }
            match v.dimensions {
                Some((w, h)) => take(w, h),
                None => take(v.visible_columns as u16, v.visible_lines as u16),
            }
        }
        match (cols, rows) {
            (Some(c), Some(r)) => Size { cols: c, rows: r }.clamped(),
            _ => self.worlds[idx].shell.as_ref().map(|s| s.size()).unwrap_or(Size::DEFAULT),
        }
    }

    /// Re-size every live shell world's PTY to its viewers (called where NAWS is sent and
    /// when a client's view changes).
    pub(crate) fn resize_shell_worlds(&mut self) {
        for idx in 0..self.worlds.len() {
            if !self.shell_live(idx) {
                continue;
            }
            let size = self.shell_dimensions(idx);
            if let Some(session) = self.worlds[idx].shell.as_ref() {
                session.resize(size);
            }
        }
    }

    /// Send any shell world's pending screen change to its viewers. Called at the bottom of
    /// every event-loop pass, so a frame goes out as soon as the loop wakes (on the Wakeup
    /// that produced it) rather than waiting on a timer. alacritty already coalesces a burst
    /// of PTY output into one Wakeup, and the loop only iterates per event, so this is
    /// naturally rate-limited without a wall-clock gate (which used to strand the tail
    /// frame of a burst).
    pub(crate) fn flush_shell_frames(&mut self) {
        for idx in 0..self.worlds.len() {
            // A live session may have more to show; read it first.
            if self.shell_live(idx) {
                self.refresh_shell_screen(idx);
            }
            if self.worlds[idx].shell_screen.as_ref().is_some_and(|s| s.has_changes()) {
                self.flush_one_shell_frame(idx);
            }
        }
    }

    /// Broadcast one delta for world `idx` to its viewers.
    fn flush_one_shell_frame(&mut self, idx: usize) {
        let Some(screen) = self.worlds[idx].shell_screen.as_mut() else { return };
        if !screen.has_changes() {
            return;
        }
        let msg = screen.delta_msg(idx);
        self.ws_broadcast_to_world(idx, msg);
    }

    /// Send a full screen frame to one client (on switch, resync, state request).
    pub(crate) fn send_shell_full_frame(&mut self, client_id: u64, idx: usize) {
        if let Some(screen) = self.worlds.get(idx).and_then(|w| w.shell_screen.as_ref()) {
            let msg = screen.full_msg(idx);
            self.ws_send_to_client(client_id, msg);
        }
    }

    /// A key, text, or paste from a client for a shell world's program.
    pub(crate) fn shell_input(&mut self, idx: usize, key: Option<ShellKey>, text: Option<String>, paste: bool) {
        let Some(session) = self.worlds.get(idx).and_then(|w| w.shell.as_ref()) else { return };
        let mode = screen::mode_msg(session.mode());
        let mut bytes = Vec::new();
        if let Some(key) = &key {
            bytes.extend(shell::keys::encode(key, &mode));
        }
        if let Some(text) = &text {
            bytes.extend(shell::keys::text_bytes(text, paste, &mode));
        }
        session.write(bytes);
    }

    /// A mouse event from a client over a shell world's grid.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn shell_mouse(&mut self, idx: usize, kind: &str, button: u8, col: u16, row: u16, ctrl: bool, alt: bool, shift: bool) {
        let Some(session) = self.worlds.get(idx).and_then(|w| w.shell.as_ref()) else { return };
        let mode = screen::mode_msg(session.mode());
        let bytes = shell::keys::mouse(kind, button, col, row, ctrl, alt, shift, &mode)
            .or_else(|| shell::keys::wheel_arrows(kind, &mode));
        if let Some(bytes) = bytes {
            session.write(bytes);
        }
    }
}
