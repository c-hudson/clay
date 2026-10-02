//! App-side application of TF effects (see `tf::effects`).
//!
//! The TF engine records everything a command does as an ordered list of effects; this
//! module applies them, in that order, in every mode. Output, errors, sends, `/recall`,
//! `/repeat`, unfinished `/quote`s and `addworld()` are applied the same way everywhere;
//! only a Clay command needs the host's own dispatcher, so the shared loop
//! (`App::apply_tf_effects_until_clay`) stops at each one and hands it to:
//!
//! - the interactive console: `commands::handle_command` (`run_tf_effects_console`)
//! - the console master answering a WebSocket client: `handle_ws_send_command_impl`
//!   (`App::run_tf_effects_client`)
//! - the headless/GUI master and `-D`: `daemon::handle_daemon_ws_message_impl`
//!   (`run_tf_effects_daemon`)
//!
//! Synchronous code (a hook fired from a connect handler, triggers inside
//! `process_server_data`) applies what it can at once and leaves the rest, from the first
//! Clay command on, for the event loop (`App::apply_tf_effects_now`,
//! `drain_deferred_tf_console`/`drain_deferred_tf_daemon`).
//!
//! A Clay command that came out of TF and still names nothing Clay knows is reported as
//! unknown here; it is never handed back to TF, which already passed on it.

use std::collections::VecDeque;

use tokio::sync::mpsc;

use crate::tf::effects::TfEffect;
use crate::{App, AppEvent, Command, WsAsyncAction, WsMessage};

/// A mail file as the last mail check found it (`App::check_tf_mail`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct MailFile {
    pub unread: bool,
    /// The error checking it, reported once until it changes.
    pub error: Option<String>,
}

/// What a run of effects belongs to.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TfEffectCtx {
    /// The world the command ran for: plain output goes here, and so do sends that name
    /// no world.
    pub world_idx: usize,
    /// Passed through to `App::emit_client_text`.
    pub daemon_mode: bool,
    /// Typed at the console's own input line: a send with no connection says so, and a
    /// started `/repeat` is announced.
    pub typed_at_console: bool,
    /// A web, GUI or remote-console client's command: it must never take over this
    /// machine's terminal (a /sh runs in the background instead).
    pub from_client: bool,
}

impl TfEffectCtx {
    /// A line the user typed at the console.
    pub(crate) fn typed(world_idx: usize) -> Self {
        TfEffectCtx { world_idx, daemon_mode: false, typed_at_console: true, from_client: false }
    }

    /// Anything else: a client's command, a trigger, a hook, a timer, a startup action.
    pub(crate) fn for_world(world_idx: usize, daemon_mode: bool) -> Self {
        TfEffectCtx { world_idx, daemon_mode, typed_at_console: false, from_client: false }
    }
}

/// How a typed line's leading slashes read, as in real tf 5.0 beta 8 (whose
/// `/help interface` text says otherwise): one or two start a command (`//x` is `/x`);
/// three or more send the line, less two slashes, to the world as text (`///x` sends
/// `/x`).
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum TypedSlashForm<'a> {
    /// Run (or send) this line as usual.
    Line(&'a str),
    /// Send this text to the world as is.
    SendText(&'a str),
}

pub(crate) fn typed_slash_form(line: &str) -> TypedSlashForm<'_> {
    if line.starts_with("///") {
        TypedSlashForm::SendText(&line[2..])
    } else if line.starts_with("//") {
        TypedSlashForm::Line(&line[1..])
    } else {
        TypedSlashForm::Line(line)
    }
}

impl App {
    /// Apply effects from the front of `queue` in order, until one is a Clay command;
    /// that command is returned for the host to run - with where it was in a file being
    /// loaded, if it came from one - and the rest stays queued.
    pub(crate) fn apply_tf_effects_until_clay(&mut self, queue: &mut VecDeque<TfEffect>, ctx: &TfEffectCtx) -> Option<(String, Option<String>)> {
        while let Some(effect) = queue.pop_front() {
            match effect {
                TfEffect::Clay { cmd, at } => return Some((cmd, at)),
                // A /connect becomes the /worlds command that connects the world - or an
                // error, TF's wording, when there's nothing to connect.
                TfEffect::Connect(req) => match self.prepare_tf_connect(req) {
                    Ok(Some(cmd)) => return Some((cmd, None)),
                    Ok(None) => {}
                    Err(e) => {
                        if !self.worlds.is_empty() {
                            let idx = ctx.world_idx.min(self.worlds.len() - 1);
                            self.emit_client_text(idx, &format!("% {}", e), ctx.daemon_mode);
                        }
                    }
                },
                other => self.apply_tf_effect(other, queue, ctx),
            }
        }
        None
    }

    /// TF's `/connect` (see `tf::builtins::cmd_connect`): find the world - the first
    /// defined one when none was named, a new temporary one for an address - set up this
    /// connection's `-l`/`-q`/`-x`, and return the `/worlds` command that connects it,
    /// in the foreground or not (None: nothing to run). Errors are TF's own.
    ///
    /// A world with no address is TF's "connectionless" kind: there is nothing to open,
    /// so `/connect` only brings it to the foreground (Clay keeps every world's output
    /// window anyway; `/echo -w` writes to it connected or not).
    pub(crate) fn prepare_tf_connect(&mut self, req: crate::tf::effects::ConnectRequest) -> Result<Option<String>, String> {
        let idx = match req.host_port {
            Some((host, port)) => self.create_temporary_world(&host, &port),
            None => {
                let name = match req.world {
                    Some(name) => name,
                    None => self.worlds.iter().find(|w| !w.is_temporary && !w.is_initial_world)
                        .map(|w| w.name.clone())
                        .ok_or_else(|| "CONNECT: default world: no such world".to_string())?,
                };
                let idx = self.find_world_index(&name).ok_or_else(|| format!("CONNECT: {}: no such world", name))?;
                if self.worlds[idx].connected {
                    return Err(format!("CONNECT: socket to {} already exists", self.worlds[idx].name));
                }
                if !self.worlds[idx].settings.has_connection_settings() {
                    return Ok((!req.background).then(|| format!("/worlds {}", self.worlds[idx].name)));
                }
                idx
            }
        };
        let world = &mut self.worlds[idx];
        world.quiet_login = req.quiet;
        world.force_ssl_once = req.ssl && !world.settings.use_ssl;
        let flags = match (req.background, req.no_login) {
            (true, true) => "-b -l ",
            (true, false) => "-b ",
            (false, true) => "-l ",
            (false, false) => "",
        };
        Ok(Some(format!("/worlds {}{}", flags, world.name)))
    }

    /// `text` shown in TF display attributes (`/echo -a`, `echo()`'s second argument,
    /// `/substitute -a`): "h", "E" and "W" are %hiliteattr, %error_attr, %warning_attr.
    /// `attrs` as the display attributes a line of `plain` text is drawn in (expanded), when
    /// there are any - kept on the line for `/recall -a` (`OutputLine::tf_attrs`).
    pub(crate) fn tf_own_attrs(&self, plain: &str, attrs: &str) -> Option<(String, crate::tf::TfAttributes)> {
        if attrs.is_empty() {
            return None;
        }
        let mut parsed = crate::tf::TfAttributes::parse(attrs).ok()?;
        let var = |name: &str| self.tf_engine.get_var(name).map(|v| v.to_string_value()).unwrap_or_default();
        parsed.expand(&var("hiliteattr"), &var("error_attr"), &var("warning_attr"));
        (!parsed.to_sgr().is_empty()).then(|| (plain.to_string(), parsed))
    }

    pub(crate) fn tf_attributed_text(&self, text: &str, attrs: &str) -> String {
        if attrs.is_empty() {
            return text.to_string();
        }
        let Ok(mut parsed) = crate::tf::TfAttributes::parse(attrs) else { return text.to_string() };
        let var = |name: &str| self.tf_engine.get_var(name).map(|v| v.to_string_value()).unwrap_or_default();
        parsed.expand(&var("hiliteattr"), &var("error_attr"), &var("warning_attr"));
        text.split('\n')
            .map(|line| crate::tf::attrs::apply_to_line(line, &parsed, &[]))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// A user set one of the special variables that stand for Clay's own settings
    /// (`special_vars::BOUND`): apply it as the setting itself - saved and sent to every
    /// client, exactly as changing it in the settings would.
    pub(crate) fn apply_tf_setting(&mut self, name: &str, value: &crate::tf::TfValue) {
        let mut changed = false;
        match name {
            "more" => {
                let on = crate::tf::special_vars::flag_is_on(value);
                if self.settings.more_mode_enabled != on {
                    self.settings.more_mode_enabled = on;
                    changed = true;
                }
            }
            "wrapspace" => {
                let n = value.tf_int().clamp(0, u8::MAX as i64) as u8;
                if self.settings.wrapspace != n {
                    self.settings.wrapspace = n;
                    self.needs_output_redraw = true;
                    changed = true;
                }
            }
            "isize" => {
                let n = value.tf_int().clamp(1, 15) as u16;
                if self.input_height != n {
                    self.input_height = n;
                    self.input.visible_height = n;
                    self.input.adjust_viewport();
                    changed = true;
                }
            }
            "insert" => self.input.insert = crate::tf::special_vars::flag_is_on(value),
            "wrap" | "wrapsize" => self.apply_tf_wrap(),
            "clock_format" => self.apply_tf_clock_format(Some(value.to_string_value())),
            "textdiv" | "textdiv_str" => self.apply_tf_textdiv(),
            _ => {}
        }
        if changed {
            let _ = crate::persistence::save_settings(self);
            let settings_msg = self.build_global_settings_msg();
            self.ws_broadcast(crate::websocket::WsMessage::GlobalSettingsUpdated { settings: settings_msg, input_height: self.input_height });
        }
        // Mirror back what Clay made of it (a clamped height, say).
        self.sync_tf_bound_settings();
    }

    /// TF's %wrap and %wrapsize, once a script sets them: output wraps at %wrapsize instead
    /// of the window's edge (unless %wrap is off) - in the console, and in every client,
    /// which caps its output's width to it.
    pub(crate) fn apply_tf_wrap(&mut self) {
        let off = self.tf_engine.get_var("wrap").is_some_and(|v| !crate::tf::special_vars::flag_is_on(v));
        let size = self.tf_engine.get_var("wrapsize").and_then(|v| v.to_int()).filter(|n| *n > 0);
        let columns = if off { 0 } else { size.unwrap_or(0) as usize };
        if self.settings.wrap_columns != columns {
            self.settings.wrap_columns = columns;
            self.needs_output_redraw = true;
            let settings_msg = self.build_global_settings_msg();
            self.ws_broadcast(crate::websocket::WsMessage::GlobalSettingsUpdated { settings: settings_msg, input_height: self.input_height });
        }
    }

    /// TF's %clock_format for Clay's own status bar clock, in every interface: `set` is a
    /// value a script just assigned; with None, the variable's value counts only when it
    /// differs from TF's default (a saved or restored one) - otherwise Clay's 12-hour
    /// clock stays.
    pub(crate) fn apply_tf_clock_format(&mut self, set: Option<String>) {
        let format = set.unwrap_or_else(|| {
            let value = self.tf_engine.get_var("clock_format").map(|v| v.to_string_value()).unwrap_or_default();
            if self.tf_engine.seed_values.get("clock_format") == Some(&value) { String::new() } else { value }
        });
        if self.settings.clock_format != format {
            self.settings.clock_format = format;
            self.needs_output_redraw = true;
            let settings_msg = self.build_global_settings_msg();
            self.ws_broadcast(crate::websocket::WsMessage::GlobalSettingsUpdated { settings: settings_msg, input_height: self.input_height });
        }
    }

    /// TF's %textdiv (and %textdiv_str) for every interface's windowed view: unset, Clay's
    /// own ▶ markers stay; set to anything, a world brought forward shows TF's divider (or,
    /// for "clear", no old text) instead - see `World::console_textdiv` and app.js.
    pub(crate) fn apply_tf_textdiv(&mut self) {
        let textdiv = self.tf_engine.get_var("textdiv").map(|v| v.to_string_value().to_lowercase()).unwrap_or_default();
        let textdiv_str = self.tf_engine.get_var("textdiv_str").map(|v| v.to_string_value()).unwrap_or_default();
        if self.settings.textdiv != textdiv || self.settings.textdiv_str != textdiv_str {
            self.settings.textdiv = textdiv;
            self.settings.textdiv_str = textdiv_str;
            self.needs_output_redraw = true;
            let settings_msg = self.build_global_settings_msg();
            self.ws_broadcast(crate::websocket::WsMessage::GlobalSettingsUpdated { settings: settings_msg, input_height: self.input_height });
        }
    }

    /// Mirror Clay's settings into the special variables that stand for them - marked as
    /// the engine's own values, so they are never written to settings.dat as TF variables.
    pub(crate) fn sync_tf_bound_settings(&mut self) {
        let values = [
            ("more", crate::tf::TfValue::String(if self.settings.more_mode_enabled { "on" } else { "off" }.to_string())),
            ("wrapspace", crate::tf::TfValue::Integer(self.settings.wrapspace as i64)),
            ("isize", crate::tf::TfValue::Integer(self.input_height as i64)),
        ];
        for (name, value) in values {
            self.tf_engine.seed_values.insert(name.to_string(), value.to_string_value());
            self.tf_engine.set_global(name, value);
        }
    }

    /// A temporary world "(unnamed<N>)" for `/connect <host> <port>`, as TF names it.
    fn create_temporary_world(&mut self, host: &str, port: &str) -> usize {
        self.last_temp_world += 1;
        let name = loop {
            let candidate = format!("(unnamed{})", self.last_temp_world);
            if self.find_world_index(&candidate).is_none() {
                break candidate;
            }
            self.last_temp_world += 1;
        };
        let mut world = crate::World::new(&name);
        world.scrollback_tx = self.scrollback.as_ref().map(|db| db.sender());
        world.is_temporary = true;
        world.temp_connect_started = Some(std::time::Instant::now());
        world.settings.hostname = host.to_string();
        world.settings.port = port.to_string();
        world.settings.auto_connect_type = crate::AutoConnectType::NoLogin;
        self.worlds.push(world);
        let idx = self.worlds.len() - 1;
        self.broadcast_world_added(idx);
        idx
    }

    /// TF: "A temporary world will be undefined when it is no longer in use" - once it is
    /// disconnected, not being connected, and in nobody's foreground. Runs once per pass
    /// of the event loop (`drain_deferred_tf_*`), never mid-command: removing a world
    /// shifts the index of every world after it.
    pub(crate) fn reap_temporary_worlds(&mut self) {
        // Long enough for a slow connect to finish; a failed one goes after this.
        const CONNECT_GRACE: std::time::Duration = std::time::Duration::from_secs(120);
        let now = std::time::Instant::now();
        let mut i = 0;
        while i < self.worlds.len() {
            let w = &self.worlds[i];
            let connecting = w.temp_connect_started.is_some_and(|t| now.duration_since(t) < CONNECT_GRACE);
            let unused = w.is_temporary && !w.connected && !connecting && w.reconnect_at.is_none()
                && i != self.current_world_index && !self.ws_client_viewing(i);
            if unused && self.worlds.len() > 1 {
                self.delete_world(i);
            } else {
                i += 1;
            }
        }
    }

    /// Run one TF line for `world_idx` - TF's current world while it runs (see
    /// `TfEngine::context_world`) - and return what it did, in order.
    pub(crate) fn run_tf_in_world(&mut self, world_idx: usize, line: &str) -> Vec<TfEffect> {
        self.sync_tf_world_info();
        let world_name = self.worlds.get(world_idx).map(|w| w.name.clone());
        self.tf_engine.with_context_world(world_name.as_deref(), |engine| engine.run(line))
    }

    /// Run a line a user typed (into the console or a client's input box) for
    /// `world_idx`, the way TF processes typed lines by `%sub` (`TfEngine::run_typed`):
    /// with the default `sub=off`, nothing in it is substituted.
    pub(crate) fn run_typed_tf_in_world(&mut self, world_idx: usize, line: &str) -> Vec<TfEffect> {
        self.sync_tf_world_info();
        let world_name = self.worlds.get(world_idx).map(|w| w.name.clone());
        self.tf_engine.with_context_world(world_name.as_deref(), |engine| engine.run_typed(line))
    }

    /// Run a key binding's command (a `/bind` body, or `/key_<name>`) for `world_idx`.
    /// TF runs a binding as a macro body: split on `%;`, fully substituted.
    pub(crate) fn run_tf_body_in_world(&mut self, world_idx: usize, body: &str) -> Vec<TfEffect> {
        self.sync_tf_world_info();
        let world_name = self.worlds.get(world_idx).map(|w| w.name.clone());
        self.tf_engine.with_context_world(world_name.as_deref(), |engine| engine.run_body(body))
    }

    /// For synchronous callers: apply effects now. From the first Clay command on, the
    /// rest is kept for the event loop, which can run Clay commands
    /// (`drain_deferred_tf_console`/`drain_deferred_tf_daemon`), so order still holds.
    pub(crate) fn apply_tf_effects_now(&mut self, effects: Vec<TfEffect>, ctx: TfEffectCtx) {
        let mut queue: VecDeque<TfEffect> = effects.into();
        if let Some((cmd, at)) = self.apply_tf_effects_until_clay(&mut queue, &ctx) {
            queue.push_front(TfEffect::Clay { cmd, at });
            self.deferred_tf.push_back((ctx, queue));
        }
    }

    /// Effects emitted while no command frame was open (see `TfEngine::take_effects`),
    /// applied for the current world.
    pub(crate) fn apply_stray_tf_effects(&mut self, daemon_mode: bool) {
        let effects = self.tf_engine.take_effects();
        if !effects.is_empty() {
            let ctx = TfEffectCtx::for_world(self.current_world_index, daemon_mode);
            self.apply_tf_effects_now(effects, ctx);
        }
    }

    fn tf_world_or(&self, world: &Option<String>, fallback: usize) -> usize {
        match world {
            Some(name) if !name.is_empty() => self.worlds.iter()
                .position(|w| w.name.eq_ignore_ascii_case(name))
                .unwrap_or(fallback),
            _ => fallback,
        }
    }

    fn apply_tf_effect(&mut self, effect: TfEffect, queue: &mut VecDeque<TfEffect>, ctx: &TfEffectCtx) {
        if self.worlds.is_empty() {
            return;
        }
        let world_idx = ctx.world_idx.min(self.worlds.len() - 1);
        match effect {
            TfEffect::Output { text, attrs, world, plain } => {
                let idx = self.tf_world_or(&world, world_idx);
                let marks = crate::LineMarks::parse(&attrs);
                // The line's own attributes: already drawn in (`/echo -a`, whose text a $()
                // keeps as it is), or drawn in here (`echo()`'s second argument).
                let (text, own) = match plain {
                    Some(plain) => (text, self.tf_own_attrs(&plain, &attrs)),
                    None => {
                        let own = self.tf_own_attrs(&text, &attrs);
                        (self.tf_attributed_text(&text, &attrs), own)
                    }
                };
                self.emit_client_text_marked(idx, &text, ctx.daemon_mode, marks, own);
            }
            // An error with a place in a loaded file prints the way TF prints it.
            TfEffect::Error { msg, at: Some(at) } => {
                self.emit_client_text(world_idx, &format!("% {}: {}", at, msg), ctx.daemon_mode);
            }
            TfEffect::Error { msg, at: None } => {
                self.emit_tf_error(world_idx, &msg, ctx.daemon_mode);
            }
            TfEffect::Send { text, world, no_eol: _ } => {
                let idx = self.tf_world_or(&world, world_idx);
                if self.send_to_world(idx, text) {
                    self.worlds[idx].last_send_time = Some(std::time::Instant::now());
                    if ctx.typed_at_console {
                        self.worlds[idx].last_user_command_time = Some(std::time::Instant::now());
                        self.worlds[idx].clear_prompt_after_send();
                        self.worlds[idx].lines_since_pause = 0;
                    }
                } else if ctx.typed_at_console {
                    self.add_output("Not connected. Use /worlds to connect.");
                }
            }
            TfEffect::Clay { cmd, at } => {
                // Only reached through apply_tf_effects_until_clay's caller contract
                // being broken; keep the command rather than lose it.
                queue.push_front(TfEffect::Clay { cmd, at });
            }
            TfEffect::Recall(opts) => {
                self.emit_recall(&opts, world_idx, ctx.daemon_mode);
            }
            TfEffect::StartProcess(process) => {
                if ctx.typed_at_console {
                    let id = process.id;
                    let interval = crate::format_duration_short(process.interval);
                    let count_str = process.count.map_or("infinite".to_string(), |c| c.to_string());
                    let cmd = process.command.clone();
                    self.register_repeat_process(process);
                    self.add_tf_output(&format!("% Process {} started: {} every {} ({} times)", id, cmd, interval, count_str));
                } else {
                    self.register_repeat_process(process);
                }
            }
            TfEffect::Quote(q) => {
                let disposition = q.disposition;
                let Some((target_idx, lines)) = self.start_quote(q, world_idx) else {
                    return;
                };
                match disposition {
                    crate::tf::QuoteDisposition::Send => {
                        if target_idx < self.worlds.len() && self.worlds[target_idx].connected {
                            for line in lines {
                                self.tf_echo_line(target_idx, "qecho", "qprefix", &line);
                                self.send_to_world_and_mark_sent(target_idx, line);
                            }
                        } else {
                            self.emit_client_text(world_idx, "Not connected", ctx.daemon_mode);
                        }
                    }
                    crate::tf::QuoteDisposition::Echo => {
                        if !lines.is_empty() {
                            self.emit_client_lines(world_idx, &lines, ctx.daemon_mode);
                        }
                    }
                    crate::tf::QuoteDisposition::Exec => {
                        // Each line runs as a TF command, unexpanded as TF runs quoted text;
                        // what they do comes next, ahead of anything already queued
                        // (depth-first, as TF would run them).
                        let mut produced = Vec::new();
                        self.sync_tf_world_info();
                        let world_name = self.worlds.get(target_idx).map(|w| w.name.clone());
                        for line in lines {
                            produced.extend(self.tf_engine.with_context_world(world_name.as_deref(), |engine| engine.run_unexpanded(&line)));
                        }
                        // Their current world is the quote's (`/help quote`: -w).
                        for effect in produced.iter_mut() {
                            if let TfEffect::Send { world: world @ None, .. } = effect {
                                *world = world_name.clone();
                            }
                        }
                        for effect in produced.into_iter().rev() {
                            queue.push_front(effect);
                        }
                    }
                }
            }
            TfEffect::WorldOp(op) => {
                crate::commands::apply_world_definition(self, op, world_idx, ctx.daemon_mode);
            }
            TfEffect::Unknown(name) => {
                self.report_unknown_tf_command(&name, None, ctx);
            }
            TfEffect::Connect(req) => {
                // As for Clay above: only reached if a caller skipped
                // apply_tf_effects_until_clay; keep it rather than lose it.
                queue.push_front(TfEffect::Connect(req));
            }
            TfEffect::Setting(name, value) => {
                self.apply_tf_setting(&name, &value);
            }
            TfEffect::Shell { command } => self.start_tf_shell(command, world_idx, ctx),
            TfEffect::SetPrompt { text, world } => {
                let idx = self.tf_world_or(&Some(world), world_idx);
                self.set_world_prompt(idx, text);
            }
            TfEffect::RecordLine { text, target, world, time } => {
                let idx = self.tf_world_or(&world, world_idx);
                self.record_tf_line(idx, text, target, time);
            }
            TfEffect::LocalEcho { on, world } => {
                // IAC DONT ECHO (we echo) / IAC DO ECHO (the server does).
                let idx = self.tf_world_or(&Some(world), world_idx);
                let verb = if on { 254 } else { 253 };
                if let Some(tx) = self.worlds[idx].command_tx.as_ref() {
                    let _ = tx.try_send(crate::WriteCommand::Raw(vec![255, verb, 1]));
                }
            }
            TfEffect::Suspend => {
                if self.console_tui && !ctx.from_client {
                    self.pending_terminal_ops.push_back(crate::TerminalOp::Suspend);
                } else {
                    self.emit_tf_error(world_idx, "SUSPEND: only Clay's own console can be suspended", ctx.daemon_mode);
                }
            }
        }
    }

    /// TF's mail check (`/help mail`): every %maildelay, each file in %TFMAILPATH (else
    /// %MAIL) has unread mail when it was written more recently than read. New mail runs
    /// the MAIL hook, after TF's "% You have new mail in <file>." unless that hook gags it;
    /// mail already there at the first check is just mentioned; an error is reported
    /// once. `nmail()` and the status line's @mail count the files with unread mail.
    /// Clay checks only once a script asks for it - a MAIL hook, @mail on a status line
    /// that is the script's, or a mail path it set - not merely because $MAIL is set.
    pub(crate) fn check_tf_mail(&mut self) {
        let now = std::time::Instant::now();
        if self.tf_mail_due.is_some_and(|due| now < due) {
            return;
        }
        let engine = &self.tf_engine;
        let delay = engine.get_var("maildelay")
            .and_then(|v| crate::tf::special_vars::dtime_secs(&v.to_string_value()))
            .unwrap_or(60.0);
        self.tf_mail_due = Some(now + std::time::Duration::from_secs_f64(delay.max(1.0)));
        let var = |name: &str| engine.get_var(name).map(|v| v.to_string_value()).filter(|s| !s.is_empty());
        let mail_env = engine.env_imported.get("MAIL").cloned();
        let asked = engine.macros.iter().any(|m| m.has_hook(crate::tf::TfHookEvent::Mail))
            || (engine.status.customized && engine.status.rows.iter().flatten().any(|f| f.name == "@mail"))
            || var("TFMAILPATH").is_some()
            || var("MAIL").is_some_and(|m| Some(&m) != mail_env.as_ref());
        if delay <= 0.0 || !asked {
            return;
        }
        let files: Vec<String> = match var("TFMAILPATH") {
            Some(path) => crate::tf::builtins::split_tf_path_list(&path),
            None => var("MAIL").into_iter().collect(),
        };
        let mut unread_count = 0;
        for name in files {
            let path = self.tf_engine.expand_tilde(&name);
            let state = match std::fs::metadata(&path) {
                Ok(meta) => {
                    let written = meta.modified().ok();
                    let read = meta.accessed().ok();
                    Ok(meta.len() > 0 && matches!((written, read), (Some(w), Some(r)) if w > r))
                }
                Err(e) => Err(e.to_string()),
            };
            let before = self.tf_mail_files.get(&name).cloned();
            match &state {
                Ok(unread) => {
                    if *unread {
                        unread_count += 1;
                    }
                    let was_unread = before.as_ref().is_some_and(|b| b.unread);
                    if *unread && !was_unread {
                        let idx = self.current_world_index;
                        if before.is_none() {
                            self.emit_client_text(idx, &format!("% You have mail in {}.", name), false);
                        } else {
                            let selected = crate::tf::hooks::select_hooks(&self.tf_engine, crate::tf::TfHookEvent::Mail, &name);
                            if !selected.first().is_some_and(|m| m.attributes.gag) {
                                self.emit_client_text(idx, &format!("% You have new mail in {}.", name), false);
                            }
                            self.fire_tf_hook(Some(idx), crate::tf::TfHookEvent::Mail, &name, false);
                        }
                    }
                }
                Err(error) => {
                    if before.as_ref().and_then(|b| b.error.as_ref()) != Some(error) {
                        let idx = self.current_world_index;
                        self.emit_client_text(idx, &format!("% {}: {}", name, error), false);
                    }
                }
            }
            self.tf_mail_files.insert(name, MailFile {
                unread: state.as_ref().is_ok_and(|u| *u),
                error: state.err(),
            });
        }
        self.tf_engine.mail_count = unread_count;
    }

    /// The rows between the output and the input area: TF's status area when a script has
    /// it (%status_height of them), else Clay's one-row bar.
    pub(crate) fn separator_rows(&self) -> u16 {
        self.tf_status.get(&self.current_world_index).map_or(1, |view| view.rows.len() as u16)
    }

    /// TF's status area (`tf::status`), once a script has made it its own: evaluated for
    /// every world - at most every quarter second, or at once when its fields change -
    /// and sent to the clients when what a world shows changes; the console draws it
    /// from `tf_status`. When it stops being a script's, every interface goes back to
    /// Clay's own bar.
    pub(crate) fn refresh_tf_status(&mut self) {
        if !self.tf_engine.status.customized {
            if !self.tf_status.is_empty() {
                self.tf_status.clear();
                self.tf_status_at = None;
                self.ws_broadcast(WsMessage::TfStatus { world_index: 0, view: None });
                self.needs_output_redraw = true;
            }
            return;
        }
        let now = std::time::Instant::now();
        if let Some((at, layout)) = &self.tf_status_at {
            if *layout == self.tf_engine.status && now.duration_since(*at) < std::time::Duration::from_millis(250) {
                return;
            }
        }
        self.tf_status_at = Some((now, self.tf_engine.status.clone()));
        self.sync_tf_world_info();
        for idx in 0..self.worlds.len() {
            let name = self.worlds[idx].name.clone();
            let view = crate::tf::status::evaluate(&mut self.tf_engine, Some(&name));
            if self.tf_status.get(&idx) != Some(&view) {
                self.tf_status.insert(idx, view.clone());
                self.ws_broadcast(WsMessage::TfStatus { world_index: idx, view: Some(view) });
                if idx == self.current_world_index {
                    self.needs_output_redraw = true;
                }
            }
        }
        let count = self.worlds.len();
        self.tf_status.retain(|idx, _| *idx < count);
    }

    /// `/sh` (`TfEffect::Shell`). On Clay's own console it gets the terminal, as in TF
    /// (the console loop runs it: `pending_terminal_ops`). Anywhere else - a web, GUI or
    /// remote-console client's command, a headless or -D Clay - there is no terminal to
    /// give: a command runs in the background and its output is shown when it is done
    /// (`AppEvent::TfShellDone`), and an interactive shell can't be had at all.
    fn start_tf_shell(&mut self, command: Option<String>, world_idx: usize, ctx: &TfEffectCtx) {
        if self.console_tui && !ctx.from_client {
            self.pending_terminal_ops.push_back(crate::TerminalOp::Shell { command, world_idx });
            return;
        }
        let Some(command) = command else {
            self.emit_tf_error(world_idx, "SH: an interactive shell needs Clay's own console; use /sh <command>", ctx.daemon_mode);
            return;
        };
        let mut cmd = shell_command(&command);
        if let Some(ref dir) = self.tf_engine.current_dir {
            cmd.current_dir(dir);
        }
        self.tf_engine.apply_child_env(&mut cmd);
        cmd.stdin(std::process::Stdio::null());
        let Some(tx) = self.tf_event_tx.clone() else {
            // No loop to report back to: run it now.
            let (output, status) = captured_shell_result(cmd.output());
            self.finish_tf_shell(world_idx, &output, status, ctx.daemon_mode);
            return;
        };
        let mut cmd = tokio::process::Command::from(cmd);
        cmd.kill_on_drop(true);
        tokio::spawn(async move {
            let (output, status) = captured_shell_result(cmd.output().await);
            let _ = tx.send(AppEvent::TfShellDone(world_idx, output, status)).await;
        });
    }

    /// A captured `/sh <command>` finished: show what it printed, and its exit status is
    /// `%?` (as /sh returns it in TF).
    pub(crate) fn finish_tf_shell(&mut self, world_idx: usize, output: &str, status: i32, daemon_mode: bool) {
        self.tf_engine.set_global("?", crate::tf::TfValue::Integer(status as i64));
        if self.worlds.is_empty() || output.is_empty() {
            return;
        }
        let idx = world_idx.min(self.worlds.len() - 1);
        self.emit_client_text(idx, output, daemon_mode);
    }

    /// Run every process that is due, once: a /repeat's body as a macro body, a /quote's
    /// next line (or all of them, for a quote with no interval). With %lpquote on, none
    /// runs on its timer - they all wait for prompts (`run_prompt_processes`). Returns
    /// what each run did, with the world it ran for, for the host loop to apply.
    /// Processes that are done are removed.
    pub(crate) fn tick_tf_processes(&mut self, now: std::time::Instant, daemon_mode: bool) -> Vec<(TfEffectCtx, Vec<TfEffect>)> {
        if self.tf_lpquote() {
            return Vec::new();
        }
        // Ids, not indices: a body can /kill or start processes while this runs.
        let due: Vec<u32> = self.tf_engine.processes.iter()
            .filter(|p| !p.on_prompt && p.next_run <= now)
            .map(|p| p.id)
            .collect();
        self.run_tf_processes(&due, false, daemon_mode)
    }

    /// A prompt arrived from `world_idx`: run, once, each process waiting for one - those
    /// started with -P, or all of them while %lpquote is on - unless it belongs to another
    /// world (`/help processes`). Returns what they did, as `tick_tf_processes` does.
    pub(crate) fn run_prompt_processes(&mut self, world_idx: usize, daemon_mode: bool) -> Vec<(TfEffectCtx, Vec<TfEffect>)> {
        let lpquote = self.tf_lpquote();
        let Some(name) = self.worlds.get(world_idx).map(|w| w.name.clone()) else { return Vec::new() };
        let due: Vec<u32> = self.tf_engine.processes.iter()
            .filter(|p| p.on_prompt || lpquote)
            .filter(|p| p.world.as_deref().is_none_or(|w| w.is_empty() || w.eq_ignore_ascii_case(&name)))
            .map(|p| p.id)
            .collect();
        self.run_tf_processes(&due, true, daemon_mode)
    }

    fn tf_lpquote(&self) -> bool {
        self.tf_engine.get_var("lpquote").is_some_and(crate::tf::special_vars::flag_is_on)
    }

    fn run_tf_processes(&mut self, ids: &[u32], on_prompt: bool, daemon_mode: bool) -> Vec<(TfEffectCtx, Vec<TfEffect>)> {
        let mut runs = Vec::new();
        for &id in ids {
            let Some(i) = self.tf_engine.processes.iter().position(|p| p.id == id) else { continue };
            let world_idx = match self.tf_engine.processes[i].world.clone() {
                Some(name) if !name.is_empty() => self.find_world_index(&name).unwrap_or(self.current_world_index),
                _ => self.current_world_index,
            };
            let p = &mut self.tf_engine.processes[i];
            if !on_prompt {
                p.next_run += p.interval;
            }
            if let Some(rem) = p.remaining.as_mut() {
                *rem = rem.saturating_sub(1);
            }
            let effects = match p.kind {
                crate::tf::ProcessKind::Quote => {
                    // Text a /quote generates is never expanded (`/help processes`).
                    let take = if p.interval.is_zero() && !p.on_prompt && !on_prompt { p.lines.len() } else { 1 };
                    let lines: Vec<String> = p.lines.drain(..take.min(p.lines.len())).collect();
                    vec![TfEffect::Quote(crate::tf::effects::QuoteRequest {
                        lines,
                        disposition: p.disposition,
                        world: p.world.clone(),
                        timing: crate::tf::QuoteTiming::Sync,
                        recall_opts: None,
                        strip_ansi: false,
                        pid: None,
                        label: String::new(),
                    })]
                }
                crate::tf::ProcessKind::Repeat => {
                    let command = p.command.clone();
                    self.sync_tf_world_info();
                    let world_name = self.worlds.get(world_idx).map(|w| w.name.clone());
                    self.tf_engine.with_context_world(world_name.as_deref(), |engine| engine.run_body(&command))
                }
            };
            runs.push((TfEffectCtx::for_world(world_idx, daemon_mode), effects));
        }
        self.tf_engine.processes.retain(|p| {
            p.remaining != Some(0) && !(p.kind == crate::tf::ProcessKind::Quote && p.lines.is_empty())
        });
        runs
    }

    /// When the next timed process is due, if any (for arming the loop's timer).
    pub(crate) fn next_tf_process_due(&self) -> Option<std::time::Instant> {
        if self.tf_lpquote() {
            return None;
        }
        self.tf_engine.processes.iter()
            .filter(|p| !p.on_prompt)
            .map(|p| p.next_run)
            .min()
    }

    /// `addworld()`: create or update a world and save.
    /// The name of an unknown command, if `cmd` - a Clay command that came out of TF -
    /// names nothing Clay can run: not a Clay command, and no action by that name for
    /// `world_idx`. It must not go back to TF, which has already passed on it.
    pub(crate) fn unknown_tf_clay_command(&self, cmd: &str, world_idx: usize) -> Option<String> {
        if let Command::ActionCommand { name, .. } = crate::parse_command(cmd) {
            let world_name = self.worlds.get(world_idx).map(|w| w.name.as_str()).unwrap_or("");
            if crate::actions::find_invocable_action(&self.settings.actions, &name, world_name).is_none() {
                return Some(name.trim_start_matches('/').to_string());
            }
        }
        None
    }

    /// NOMACRO hook, then "Unknown command" - TF-style with the file and line when the
    /// command came from a file being loaded (`at`).
    pub(crate) fn report_unknown_tf_command(&mut self, name: &str, at: Option<&str>, ctx: &TfEffectCtx) {
        if self.worlds.is_empty() {
            return;
        }
        let world_idx = ctx.world_idx.min(self.worlds.len() - 1);
        self.fire_tf_hook(Some(world_idx), crate::tf::TfHookEvent::Nomacro, name, ctx.daemon_mode);
        let name = name.trim_start_matches('/');
        let text = match at {
            Some(at) => format!("% {}: Unknown command: /{}", at, name),
            None => format!("Unknown command: /{}", name),
        };
        self.emit_client_text(world_idx, &text, ctx.daemon_mode);
    }

    /// Master console answering a WebSocket client: apply effects, running each Clay
    /// command through this client's own command path. Only one async follow-up can be
    /// returned to the caller; a further connect is queued as a `ConnectWorldRequest`.
    pub(crate) fn run_tf_effects_client(&mut self, client_id: u64, effects: Vec<TfEffect>, ctx: TfEffectCtx, event_tx: &mpsc::Sender<AppEvent>) -> WsAsyncAction {
        let ctx = TfEffectCtx { from_client: true, ..ctx };
        let mut queue: VecDeque<TfEffect> = effects.into();
        let mut first_action = WsAsyncAction::Done;
        while let Some((cmd, at)) = self.apply_tf_effects_until_clay(&mut queue, &ctx) {
            if let Some(name) = self.unknown_tf_clay_command(&cmd, ctx.world_idx) {
                self.report_unknown_tf_command(&name, at.as_deref(), &ctx);
                continue;
            }
            let action = self.handle_ws_send_command_impl(client_id, ctx.world_idx, &cmd, event_tx);
            match action {
                WsAsyncAction::Done => {}
                other if matches!(first_action, WsAsyncAction::Done) => first_action = other,
                WsAsyncAction::Connect { world_index, .. } => {
                    let _ = event_tx.try_send(AppEvent::ConnectWorldRequest(world_index, String::new()));
                }
                _ => {}
            }
        }
        first_action
    }
}

/// Interactive console: apply effects, running Clay commands with `handle_command`.
/// Returns true when a command asked to quit.
///
/// A trigger's or hook's command runs with its own world as the current one (a `/send`
/// with no `-w` goes there), as it always has; the console's previous world comes back
/// afterwards unless the command itself switched worlds.
pub(crate) async fn run_tf_effects_console(app: &mut App, effects: Vec<TfEffect>, ctx: TfEffectCtx, event_tx: &mpsc::Sender<AppEvent>) -> bool {
    let mut queue: VecDeque<TfEffect> = effects.into();
    run_tf_queue_console(app, &mut queue, ctx, event_tx).await
}

async fn run_tf_queue_console(app: &mut App, queue: &mut VecDeque<TfEffect>, ctx: TfEffectCtx, event_tx: &mpsc::Sender<AppEvent>) -> bool {
    while let Some((cmd, at)) = app.apply_tf_effects_until_clay(queue, &ctx) {
        if let Some(name) = app.unknown_tf_clay_command(&cmd, ctx.world_idx) {
            app.report_unknown_tf_command(&name, at.as_deref(), &ctx);
            continue;
        }
        let saved = app.current_world_index;
        let swap = !ctx.typed_at_console && ctx.world_idx < app.worlds.len() && ctx.world_idx != saved;
        if swap {
            app.current_world_index = ctx.world_idx;
        }
        let quit = Box::pin(crate::commands::handle_command(&cmd, app, event_tx.clone())).await;
        if swap && app.current_world_index == ctx.world_idx && saved < app.worlds.len() {
            app.current_world_index = saved;
        }
        if quit {
            return true;
        }
    }
    false
}

/// Console: run one line typed at the input area through TF, the way TF processes typed
/// lines (by `%sub`, see `TfEngine::run_typed`), and apply what it did. Returns true
/// when it asked to quit.
pub(crate) async fn run_tf_line_console(app: &mut App, line: &str, event_tx: &mpsc::Sender<AppEvent>) -> bool {
    let world_idx = app.current_world_index;
    let effects = app.run_typed_tf_in_world(world_idx, line);
    let ctx = TfEffectCtx::typed(world_idx);
    run_tf_effects_console(app, effects, ctx, event_tx).await
}

/// Console: run a key binding's command as a macro body (see `run_tf_body_in_world`).
pub(crate) async fn run_tf_binding_console(app: &mut App, body: &str, event_tx: &mpsc::Sender<AppEvent>) -> bool {
    let world_idx = app.current_world_index;
    let effects = app.run_tf_body_in_world(world_idx, body);
    let ctx = TfEffectCtx::for_world(world_idx, false);
    run_tf_effects_console(app, effects, ctx, event_tx).await
}

/// `sh -c <command>` with standard error joined to standard output, in order (as TF's
/// captured output has it); `cmd /C` on Windows.
fn shell_command(command: &str) -> std::process::Command {
    #[cfg(unix)]
    {
        let mut cmd = std::process::Command::new("/bin/sh");
        cmd.arg("-c").arg(format!("exec 2>&1\n{}", command));
        cmd
    }
    #[cfg(not(unix))]
    {
        let mut cmd = std::process::Command::new("cmd");
        cmd.arg("/C").arg(command);
        cmd
    }
}

/// The text and exit status of a captured `/sh` (-1 if it didn't exit normally).
fn captured_shell_result(result: std::io::Result<std::process::Output>) -> (String, i32) {
    match result {
        Ok(output) => {
            let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
            text.push_str(&String::from_utf8_lossy(&output.stderr));
            (text, output.status.code().unwrap_or(-1))
        }
        Err(e) => (format!("% SH: {}", e), -1),
    }
}

/// Console: run what synchronous code left for the event loop.
pub(crate) async fn drain_deferred_tf_console(app: &mut App, event_tx: &mpsc::Sender<AppEvent>) -> bool {
    if let Some((columns, lines)) = app.tf_resize.take() {
        app.fire_tf_hook(None, crate::tf::TfHookEvent::Resize, &format!("{} {}", columns, lines), false);
    }
    app.apply_stray_tf_effects(false);
    while let Some((ctx, mut queue)) = app.deferred_tf.pop_front() {
        if run_tf_queue_console(app, &mut queue, ctx, event_tx).await {
            return true;
        }
    }
    app.reap_temporary_worlds();
    false
}

/// Headless/GUI master and `-D`: apply effects, running Clay commands through the
/// daemon's client command path on behalf of `client_id` (0 = no particular client).
pub(crate) async fn run_tf_effects_daemon(app: &mut App, client_id: u64, effects: Vec<TfEffect>, ctx: TfEffectCtx, event_tx: &mpsc::Sender<AppEvent>) {
    let mut queue: VecDeque<TfEffect> = effects.into();
    run_tf_queue_daemon(app, client_id, &mut queue, ctx, event_tx).await;
}

async fn run_tf_queue_daemon(app: &mut App, client_id: u64, queue: &mut VecDeque<TfEffect>, ctx: TfEffectCtx, event_tx: &mpsc::Sender<AppEvent>) {
    while let Some((cmd, at)) = app.apply_tf_effects_until_clay(queue, &ctx) {
        if let Some(name) = app.unknown_tf_clay_command(&cmd, ctx.world_idx) {
            app.report_unknown_tf_command(&name, at.as_deref(), &ctx);
            continue;
        }
        // No particular client (a trigger, hook or timer): /notify and /say go to every
        // client, not just the one the daemon path would answer.
        if client_id == 0 && app.handle_triggered_notify_or_say(crate::parse_command(&cmd), ctx.world_idx) {
            continue;
        }
        Box::pin(crate::daemon::handle_daemon_ws_message_impl(
            app, client_id, WsMessage::SendCommand { world_index: ctx.world_idx, command: cmd }, event_tx,
        )).await;
    }
}

/// Headless/GUI master and `-D`: run what synchronous code left for the event loop.
pub(crate) async fn drain_deferred_tf_daemon(app: &mut App, event_tx: &mpsc::Sender<AppEvent>) {
    app.apply_stray_tf_effects(true);
    while let Some((ctx, mut queue)) = app.deferred_tf.pop_front() {
        run_tf_queue_daemon(app, 0, &mut queue, ctx, event_tx).await;
    }
    app.reap_temporary_worlds();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::World;

    fn app_with_world() -> App {
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("Test"));
        app.current_world_index = 0;
        app
    }

    fn texts(app: &App) -> Vec<String> {
        app.worlds[0].output_lines.iter().map(|l| l.text.clone()).collect()
    }

    /// A due process runs once, as a macro body (split on %;), and is advanced; one that
    /// is not due yet stays put.
    #[test]
    fn test_tick_runs_due_processes_as_macro_bodies() {
        let mut app = app_with_world();
        let now = std::time::Instant::now();
        let make = |id: u32, command: &str, next_run: std::time::Instant| crate::tf::TfProcess {
            id,
            command: command.to_string(),
            interval: std::time::Duration::from_millis(500),
            count: Some(2),
            remaining: Some(2),
            next_run,
            world: None,
            synchronous: false,
            on_prompt: false,
            priority: 0,
            kind: crate::tf::ProcessKind::Repeat,
            interval_given: true,
            lines: Default::default(),
            disposition: Default::default(),
        };
        app.register_repeat_process(make(1, "/echo a%; /echo b", now));
        app.register_repeat_process(make(2, "/echo later", now + std::time::Duration::from_secs(60)));

        let runs = app.tick_tf_processes(now, false);
        assert_eq!(runs.len(), 1);
        let (_, effects) = &runs[0];
        let outputs: Vec<&str> = effects.iter().filter_map(|e| match e {
            TfEffect::Output { text, .. } => Some(text.as_str()),
            _ => None,
        }).collect();
        assert_eq!(outputs, vec!["a", "b"]);
        let p = app.tf_engine.processes.iter().find(|p| p.id == 1).unwrap();
        assert_eq!(p.remaining, Some(1));
        assert_eq!(p.next_run, now + std::time::Duration::from_millis(500));
        assert_eq!(app.next_tf_process_due(), Some(now + std::time::Duration::from_millis(500)));

        // The second run uses up the count and removes the process.
        let _ = app.tick_tf_processes(now + std::time::Duration::from_millis(500), false);
        assert!(app.tf_engine.processes.iter().all(|p| p.id != 1));
    }

    /// Synchronous code applies output at once and leaves everything from the first Clay
    /// command on for the event loop, in order.
    #[test]
    fn test_apply_now_defers_from_the_first_clay_command() {
        let mut app = app_with_world();
        let effects = vec![
            TfEffect::Output { text: "first".into(), attrs: String::new(), world: None, plain: None },
            TfEffect::clay("/tag"),
            TfEffect::Output { text: "second".into(), attrs: String::new(), world: None, plain: None },
        ];
        app.apply_tf_effects_now(effects, TfEffectCtx::for_world(0, false));
        assert!(texts(&app).iter().any(|t| t == "first"));
        assert!(!texts(&app).iter().any(|t| t == "second"));
        assert_eq!(app.deferred_tf.len(), 1);
        assert_eq!(app.deferred_tf[0].1.len(), 2);
    }

    /// A Clay command from TF that names nothing Clay knows is reported, never handed
    /// back to TF.
    #[test]
    fn test_unknown_clay_command_from_tf_is_detected() {
        let app = app_with_world();
        assert_eq!(app.unknown_tf_clay_command("/zzz_nothing x", 0), Some("zzz_nothing".to_string()));
        assert_eq!(app.unknown_tf_clay_command("/tag", 0), None);
        assert_eq!(app.unknown_tf_clay_command("/worlds", 0), None);
    }

    /// The console applies a macro's effects in order, runs its Clay commands, and
    /// reports an unknown one without looping back into TF.
    #[tokio::test]
    async fn test_console_runs_effects_in_order() {
        let mut app = app_with_world();
        let (event_tx, _event_rx) = mpsc::channel::<AppEvent>(16);
        let show_tags_before = app.show_tags;
        app.tf_engine.execute("/def m = /echo before%; /tag%; /zzz_nothing%; /echo after");
        let quit = run_tf_line_console(&mut app, "/m", &event_tx).await;
        assert!(!quit);
        assert_ne!(app.show_tags, show_tags_before, "/tag must have run");
        let t = texts(&app);
        let pos = |needle: &str| t.iter().position(|l| l.contains(needle));
        let (before, unknown, after) = (pos("before"), pos("Unknown command: /zzz_nothing"), pos("after"));
        assert!(before.is_some() && unknown.is_some() && after.is_some(), "{t:?}");
        assert!(before < unknown && unknown < after, "{t:?}");
    }

    fn connect_requests(effects: &[TfEffect]) -> Vec<crate::tf::effects::ConnectRequest> {
        effects.iter().filter_map(|e| match e {
            TfEffect::Connect(req) => Some(req.clone()),
            _ => None,
        }).collect()
    }

    /// /connect's options and arguments, and TF's foreground rule: typed, it comes to the
    /// foreground; from a hook running for a background world, it doesn't (-f overrides).
    #[test]
    fn test_connect_parses_options_and_picks_foreground() {
        let mut app = app_with_world();
        app.worlds.push(World::new("Other"));
        let typed = app.run_typed_tf_in_world(0, "/connect -lqx Other");
        assert_eq!(connect_requests(&typed), vec![crate::tf::effects::ConnectRequest {
            world: Some("Other".into()), no_login: true, quiet: true, ssl: true, ..Default::default()
        }]);
        let addr = app.run_typed_tf_in_world(0, "/connect -b mud.example.com 4000");
        assert_eq!(connect_requests(&addr)[0].host_port, Some(("mud.example.com".into(), "4000".into())));
        assert!(connect_requests(&addr)[0].background);
        // A DISCONNECT-style hook running for Other (a background world) reconnects it in
        // the background - unless -f.
        app.sync_tf_world_info();
        let bg = app.tf_engine.with_context_world(Some("Other"), |e| e.run_body("/connect Other"));
        assert!(connect_requests(&bg)[0].background);
        let fg = app.tf_engine.with_context_world(Some("Other"), |e| e.run_body("/connect -f Other"));
        assert!(!connect_requests(&fg)[0].background);
        let mine = app.tf_engine.with_context_world(Some("Test"), |e| e.run_body("/connect Other"));
        assert!(!connect_requests(&mine)[0].background, "a hook for the foreground world is in the foreground");
    }

    /// The App turns /connect into the /worlds command that connects the world, with TF's
    /// errors for a world that doesn't exist or is already open.
    #[test]
    fn test_prepare_tf_connect() {
        use crate::tf::effects::ConnectRequest;
        let mut app = app_with_world();
        app.worlds[0].settings.hostname = "localhost".into();
        app.worlds[0].settings.port = "4000".into();
        app.worlds.push(World::new("Second"));
        app.worlds[1].settings.hostname = "localhost".into();
        app.worlds[1].settings.port = "4001".into();

        let req = |world: Option<&str>| ConnectRequest { world: world.map(str::to_string), ..Default::default() };
        assert_eq!(app.prepare_tf_connect(req(Some("second"))), Ok(Some("/worlds Second".to_string())));
        assert_eq!(app.prepare_tf_connect(req(None)), Ok(Some("/worlds Test".to_string())), "the first defined world");
        assert_eq!(app.prepare_tf_connect(req(Some("nosuch"))), Err("CONNECT: nosuch: no such world".to_string()));
        app.worlds[1].connected = true;
        assert_eq!(app.prepare_tf_connect(req(Some("Second"))), Err("CONNECT: socket to Second already exists".to_string()));
        let flags = ConnectRequest { world: Some("Test".into()), no_login: true, background: true, quiet: true, ssl: true, ..Default::default() };
        assert_eq!(app.prepare_tf_connect(flags), Ok(Some("/worlds -b -l Test".to_string())));
        assert!(app.worlds[0].quiet_login && app.worlds[0].force_ssl_once);

        // A world with no address ("connectionless"): just brought to the foreground.
        app.worlds.push(World::new("Window"));
        assert_eq!(app.prepare_tf_connect(req(Some("Window"))), Ok(Some("/worlds Window".to_string())));
        let bg = ConnectRequest { world: Some("Window".into()), background: true, ..Default::default() };
        assert_eq!(app.prepare_tf_connect(bg), Ok(None));

        let mut empty = App::new();
        empty.worlds.clear();
        empty.worlds.push(World::new("placeholder"));
        empty.worlds[0].is_initial_world = true;
        assert_eq!(empty.prepare_tf_connect(req(None)), Err("CONNECT: default world: no such world".to_string()));
    }

    /// /connect <host> <port> makes a temporary "(unnamed<N>)" world: never saved, kept
    /// while connecting, connected or in the foreground, removed once none of those.
    #[test]
    fn test_temporary_world_lifecycle() {
        use crate::tf::effects::ConnectRequest;
        let mut app = app_with_world();
        let req = ConnectRequest { host_port: Some(("127.0.0.1".into(), "4000".into())), background: true, ..Default::default() };
        assert_eq!(app.prepare_tf_connect(req.clone()), Ok(Some("/worlds -b (unnamed1)".to_string())));
        assert_eq!(app.prepare_tf_connect(req), Ok(Some("/worlds -b (unnamed2)".to_string())));
        assert!(app.worlds[1].is_temporary && app.worlds[2].is_temporary);

        let saved = crate::persistence::serialize_settings_for_export(&app);
        assert!(!saved.contains("unnamed"), "a temporary world is never saved");

        app.reap_temporary_worlds();
        assert_eq!(app.worlds.len(), 3, "still connecting");
        app.worlds[1].temp_connect_started = None;
        app.worlds[2].temp_connect_started = None;
        app.worlds[2].connected = true;
        app.current_world_index = 0;
        app.reap_temporary_worlds();
        assert_eq!(app.worlds.iter().map(|w| w.name.as_str()).collect::<Vec<_>>(), vec!["Test", "(unnamed2)"]);
        // Disconnected but in the foreground: kept until left.
        app.worlds[1].connected = false;
        app.current_world_index = 1;
        app.reap_temporary_worlds();
        assert_eq!(app.worlds.len(), 2);
        app.current_world_index = 0;
        app.reap_temporary_worlds();
        assert_eq!(app.worlds.len(), 1);
    }

    /// stdlib.tf's flag commands and `/set <name>`, checked against real tf.
    #[test]
    fn test_flag_commands_and_set_display() {
        let mut engine = crate::tf::TfEngine::new();
        let shown = |engine: &mut crate::tf::TfEngine, line: &str| -> Vec<String> {
            engine.run(line).into_iter().filter_map(|e| match e {
                TfEffect::Output { text, .. } => Some(text),
                TfEffect::Error { msg, .. } => Some(format!("error: {}", msg)),
                _ => None,
            }).collect()
        };
        assert!(shown(&mut engine, "/login off").is_empty());
        assert_eq!(shown(&mut engine, "/login"), vec!["% login=off"]);
        assert!(shown(&mut engine, "/login on").is_empty());
        assert!(shown(&mut engine, "/nologin").is_empty());
        assert_eq!(engine.get_var("login").map(|v| v.to_string_value()).as_deref(), Some("off"));
        assert_eq!(shown(&mut engine, "/set nosuchvar"), vec!["% nosuchvar not set globally"]);
        assert_eq!(shown(&mut engine, "/login maybe"), vec!["error: Invalid login value \"maybe\".  Valid values are: off (0), on (1)"]);
        assert_eq!(shown(&mut engine, "/kecho >> "), vec!["% kprefix=>>"]);
        assert_eq!(engine.get_var("kecho").map(|v| v.to_string_value()).as_deref(), Some("on"));
        assert!(shown(&mut engine, "/kecho off").is_empty());
        assert!(shown(&mut engine, "/isize 4").is_empty());
        assert_eq!(engine.get_var("isize").map(|v| v.to_string_value()).as_deref(), Some("4"));
    }

    /// %hook off silences every hook; %kecho shows what is typed, behind %kprefix; %lp
    /// makes every MUD world take a silent partial line as its prompt after %prompt_wait.
    #[test]
    fn test_hook_kecho_and_lp_flags() {
        let mut app = app_with_world();
        app.tf_engine.execute("/def -hCONNECT onconn = /echo HOOKED");
        app.tf_engine.execute("/set hook=off");
        app.fire_tf_hook(Some(0), crate::tf::TfHookEvent::Connect, "Test", false);
        assert!(!texts(&app).iter().any(|t| t == "HOOKED"));
        app.tf_engine.execute("/set hook=on");
        app.fire_tf_hook(Some(0), crate::tf::TfHookEvent::Connect, "Test", false);
        assert!(texts(&app).iter().any(|t| t == "HOOKED"));

        app.tf_engine.execute("/set kecho=on");
        app.tf_engine.execute("/set kprefix=>> ");
        // (Never within the login window after a connect, where a password is typed.)
        app.worlds[0].login_capture_guard = 0;
        let (event_tx, _event_rx) = mpsc::channel::<AppEvent>(16);
        let _ = app.handle_ws_send_command_impl(1, 0, "look", &event_tx);
        assert!(texts(&app).iter().any(|t| t == ">> look"), "{:?}", texts(&app));

        assert_eq!(app.timed_prompt_wait(&app.worlds[0]), None, "a plain MUD world doesn't infer prompts");
        app.tf_engine.execute("/set lp=on");
        assert_eq!(app.timed_prompt_wait(&app.worlds[0]), Some(std::time::Duration::from_millis(250)));
        app.tf_engine.execute("/set prompt_wait=1.5");
        assert_eq!(app.timed_prompt_wait(&app.worlds[0]), Some(std::time::Duration::from_millis(1500)));
    }

    /// TF's DISCONNECT hook gets real tf's arguments - the world alone when the server
    /// closed the connection, "<world> recv <error>" after a failed read - and Clay's
    /// message goes out as server text, unless the hook is gagged, which hides
    /// "Disconnected." too. A stale connection (replaced by /dc or a reconnect) says nothing.
    #[test]
    fn test_disconnect_hook_reason_and_gag() {
        use crate::telnet_reader::CloseReason;
        let mut app = app_with_world();
        app.worlds[0].connected = true;
        app.worlds[0].connection_id = 7;
        app.tf_engine.execute("/def -hDISCONNECT ondc = /echo HOOK-DISCONNECT=[%*]");
        let reset = CloseReason::ReadError("Connection reset by peer".to_string());
        match app.close_notice("Test", 7, &reset) {
            Some(AppEvent::ServerData(name, bytes)) => {
                assert_eq!(name, "Test");
                assert_eq!(bytes, b"Read error: Connection reset by peer\n");
            }
            _ => panic!("expected the close message as server text"),
        }
        assert!(app.close_notice("Test", 6, &reset).is_none(), "a stale connection says nothing");
        // A partial line held for its trigger check is completed first.
        app.worlds[0].trigger_partial_line = "half a line".to_string();
        match app.close_notice("Test", 7, &CloseReason::ByServer) {
            Some(AppEvent::ServerData(_, bytes)) => assert_eq!(bytes, b"\nConnection closed by server.\n"),
            _ => panic!("expected the close message"),
        }
        app.worlds[0].trigger_partial_line.clear();
        app.handle_disconnected(0, &reset);
        let t = texts(&app);
        assert!(t.iter().any(|t| t == "HOOK-DISCONNECT=[Test recv Connection reset by peer]"), "{:?}", t);
        assert!(t.iter().any(|t| t == "Disconnected."), "{:?}", t);

        // A server close: the world alone. A gagged hook hides both of Clay's messages.
        let mut app = app_with_world();
        app.worlds[0].connected = true;
        app.tf_engine.execute("/def -ag -hDISCONNECT ondc = /echo GAGGED-HOOK=[%*]");
        let id = app.worlds[0].connection_id;
        assert!(app.close_notice("Test", id, &CloseReason::ByServer).is_none(), "gagged: no message");
        app.handle_disconnected(0, &CloseReason::ByServer);
        let t = texts(&app);
        assert!(t.iter().any(|t| t == "GAGGED-HOOK=[Test]"), "{:?}", t);
        assert!(!t.iter().any(|t| t == "Disconnected."), "{:?}", t);
    }

    /// A world-restricted (-w) hook is judged against the world the event is about, not
    /// the one in the foreground: a background world's gagged DISCONNECT hides its message.
    #[test]
    fn test_hook_gag_follows_the_event_world() {
        use crate::telnet_reader::CloseReason;
        let mut app = app_with_world();
        app.worlds.push(World::new("Other"));
        app.worlds[1].connected = true;
        app.current_world_index = 0;
        app.tf_engine.execute("/def -ag -wOther -hDISCONNECT quiet_other = /echo x");
        assert!(app.tf_hook_gagged_for(Some(1), crate::tf::TfHookEvent::Disconnect, "Other"));
        assert!(!app.tf_hook_gagged_for(Some(0), crate::tf::TfHookEvent::Disconnect, "Test"));
        let id = app.worlds[1].connection_id;
        assert!(app.close_notice("Other", id, &CloseReason::ByServer).is_none());
    }

    /// /dc really closes the connection: its writer is told to shut down (which stops the
    /// reader too) and the connection id moves on, so what the old reader still reports
    /// is stale. TF's /dc fires no DISCONNECT hook.
    #[test]
    fn test_dc_shuts_the_connection_down() {
        use crate::telnet_reader::CloseReason;
        let mut app = app_with_world();
        let (tx, mut rx) = mpsc::channel::<crate::telnet::WriteCommand>(4);
        app.worlds[0].connected = true;
        app.worlds[0].command_tx = Some(tx);
        app.tf_engine.execute("/def -hDISCONNECT ondc = /echo FIRED");
        let id = app.worlds[0].connection_id;
        crate::commands::execute_disconnect_command(&mut app, &None, 0, false);
        assert!(matches!(rx.try_recv(), Ok(crate::telnet::WriteCommand::Shutdown)));
        assert_ne!(app.worlds[0].connection_id, id);
        assert!(!app.worlds[0].connected);
        assert!(app.close_notice("Test", id, &CloseReason::SendFailed).is_none(), "the old reader's report is stale");
        let t = texts(&app);
        assert!(t.iter().any(|t| t == "Disconnected."), "{:?}", t);
        assert!(!t.iter().any(|t| t == "FIRED"), "/dc fires no DISCONNECT: {:?}", t);
    }

    /// CONFAIL and ICONFAIL get real tf's "<world> <address> <port>: <reason>" on the
    /// client/GUI/-D connect path too (it used to fire neither, so tf-lib's /retry stopped
    /// after one failure there); Clay says "Connection failed: <reason>" once, unless the
    /// CONFAIL hook is gagged. A connect that succeeds on a later address fires its
    /// ICONFAILs first.
    #[test]
    fn test_connect_failure_hooks() {
        use crate::daemon::{AddrFailure, ConnectFailure};
        let mut app = app_with_world();
        app.tf_engine.execute("/def -hCONFAIL cf = /echo CONFAIL=[%*]");
        app.tf_engine.execute("/def -hICONFAIL icf = /echo ICONFAIL=[%*]");
        let failure = ConnectFailure { attempts: vec![
            AddrFailure::new("127.0.0.1", "4000", "Connection refused"),
            AddrFailure::new("::1", "4000", "Network is unreachable"),
        ] };
        let id = app.worlds[0].connection_id;
        app.handle_world_connect_result("Test", id, crate::ConnectOrigin::Client { report_failure: true }, Err(failure.clone()));
        let t = texts(&app);
        let hooks: Vec<&String> = t.iter().filter(|t| t.contains("CONFAIL=")).collect();
        assert_eq!(hooks, ["ICONFAIL=[Test 127.0.0.1 4000: Connection refused]", "CONFAIL=[Test ::1 4000: Network is unreachable]"]);
        // Clay's own account prefers the IPv4 address's error, and says it once.
        assert_eq!(t.iter().filter(|t| t.starts_with("Connection failed")).collect::<Vec<_>>(), ["Connection failed: Connection refused"]);

        let mut app = app_with_world();
        app.tf_engine.execute("/def -ag -hCONFAIL cf = /echo QUIET=[%*]");
        let refused = ConnectFailure::at("127.0.0.1", "1", "Connection refused");
        let id = app.worlds[0].connection_id;
        app.handle_world_connect_result("Test", id, crate::ConnectOrigin::Console, Err(refused));
        let t = texts(&app);
        assert!(t.iter().any(|t| t == "QUIET=[Test 127.0.0.1 1: Connection refused]"), "{:?}", t);
        assert!(!t.iter().any(|t| t.starts_with("Connection failed")), "a gagged CONFAIL hides it: {:?}", t);

        let mut app = app_with_world();
        app.tf_engine.execute("/def -hICONFAIL icf = /echo ICONFAIL=[%*]");
        let id = app.worlds[0].connection_id;
        let first = vec![AddrFailure::new("10.0.0.1", "23", "Connection timed out")];
        assert!(app.preprocess_event(AppEvent::ConnectAttemptsFailed("Test".into(), id, first.clone())).is_none());
        assert!(app.preprocess_event(AppEvent::ConnectAttemptsFailed("Test".into(), id + 1, first)).is_none());
        assert_eq!(texts(&app).iter().filter(|t| t.starts_with("ICONFAIL=")).count(), 1, "a stale attempt fires nothing");
    }

    /// %textdiv, once a script sets it, replaces Clay's ▶ markers in every interface (the
    /// setting reaches the clients); /unset gives them back - as /unset of %clock_format
    /// gives Clay's own clock back, which it used not to.
    #[test]
    fn test_textdiv_setting_and_unset() {
        let mut app = app_with_world();
        app.settings.new_line_indicator = true;
        app.apply_tf_textdiv();
        assert_eq!(app.settings.textdiv, "", "unset: Clay's own markers");
        assert!(app.settings.nli_drawn());
        let effects = app.tf_engine.run_typed("/set textdiv=always");
        app.apply_tf_effects_now(effects, TfEffectCtx::for_world(0, false));
        assert_eq!(app.settings.textdiv, "always");
        assert!(!app.settings.nli_drawn(), "TF's divider takes the markers' place");
        assert!(app.build_global_settings_msg().textdiv == "always");
        let effects = app.tf_engine.run_typed("/unset textdiv");
        app.apply_tf_effects_now(effects, TfEffectCtx::for_world(0, false));
        assert_eq!(app.settings.textdiv, "");
        assert!(app.settings.nli_drawn());

        let effects = app.tf_engine.run_typed("/set clock_format=%H:%M:%S");
        app.apply_tf_effects_now(effects, TfEffectCtx::for_world(0, false));
        assert_eq!(app.settings.clock_format, "%H:%M:%S");
        let effects = app.tf_engine.run_typed("/unset clock_format");
        app.apply_tf_effects_now(effects, TfEffectCtx::for_world(0, false));
        assert_eq!(app.settings.clock_format, "", "back to Clay's own clock");
    }

    /// /echo -ag, as in tf: the line isn't shown, but it is kept - /recall -ag finds it.
    #[test]
    fn test_echo_gag_keeps_the_line_hidden() {
        let mut app = app_with_world();
        app.worlds[0].showing_splash = false;
        let effects = app.run_tf_in_world(0, "/echo -ag gagged-echo");
        app.apply_tf_effects_now(effects, TfEffectCtx::for_world(0, false));
        let line = app.worlds[0].output_lines.iter().find(|l| l.text == "gagged-echo").expect("kept");
        assert!(line.gagged, "not shown");
        let found = |app: &App, show_gagged: bool| -> Vec<String> {
            let opts = crate::tf::RecallOptions { source: crate::tf::RecallSource::Local, show_gagged, ..Default::default() };
            app.recall_matches(&opts, 0).unwrap().into_iter().map(|(t, _)| t).collect()
        };
        assert!(!found(&app, false).iter().any(|t| t.contains("gagged-echo")));
        assert!(found(&app, true).iter().any(|t| t.contains("gagged-echo")));
    }

    /// Leading slashes on a typed line, as in real tf: one or two run a command, three or
    /// more send the line less two slashes.
    #[test]
    fn test_typed_slash_form() {
        assert_eq!(typed_slash_form("/echo x"), TypedSlashForm::Line("/echo x"));
        assert_eq!(typed_slash_form("//echo x"), TypedSlashForm::Line("/echo x"));
        assert_eq!(typed_slash_form("///y"), TypedSlashForm::SendText("/y"));
        assert_eq!(typed_slash_form("////q"), TypedSlashForm::SendText("//q"));
        assert_eq!(typed_slash_form("say http://x"), TypedSlashForm::Line("say http://x"));
    }

    /// A client's "///look" goes to the world as "/look"; "//echo hi" runs /echo.
    #[test]
    fn test_client_slash_forms_follow_tf() {
        let mut app = app_with_world();
        let (tx, mut rx) = mpsc::channel::<crate::WriteCommand>(8);
        app.worlds[0].command_tx = Some(tx);
        app.worlds[0].connected = true;
        let (event_tx, _event_rx) = mpsc::channel::<AppEvent>(16);
        let _ = app.handle_ws_send_command_impl(1, 0, "///look", &event_tx);
        match rx.try_recv() {
            Ok(crate::WriteCommand::Text(text)) => assert_eq!(text, "/look"),
            other => panic!("expected /look to be sent, got {:?}", other.map(|_| ())),
        }
        let _ = app.handle_ws_send_command_impl(1, 0, "//echo hi", &event_tx);
        assert!(texts(&app).iter().any(|t| t == "hi"), "{:?}", texts(&app));
        assert!(rx.try_recv().is_err(), "//echo must not reach the world");
    }

    /// Clay's own bar stays until a script changes TF's status area; from then on each
    /// world's view is evaluated, sent to the clients (and in InitialState), and the
    /// console gives it %status_height rows.
    #[test]
    fn test_tf_status_area_reaches_every_interface() {
        let mut app = app_with_world();
        let sent = |app: &App| app.ws_broadcast_log.lock().unwrap().iter()
            .filter(|m| matches!(m, crate::WsMessage::TfStatus { .. })).count();
        app.refresh_tf_status();
        assert!(app.tf_status.is_empty());
        assert_eq!(app.separator_rows(), 1);
        assert_eq!(sent(&app), 0, "nothing until a script has the status area");

        app.tf_engine.execute("/status_add -c \"HP:\" hp:4 :1 @world");
        app.tf_engine.execute("/set hp=42");
        app.refresh_tf_status();
        let view = app.tf_status.get(&0).expect("the world's view").clone();
        assert_eq!(view.rows.len(), 1);
        assert_eq!(view.rows[0][1].text, "42");
        assert_eq!(sent(&app), 1);
        app.refresh_tf_status();
        assert_eq!(sent(&app), 1, "unchanged: not sent again");
        match app.build_initial_state(0) {
            crate::WsMessage::InitialState { tf_status, .. } => {
                assert_eq!(tf_status.len(), 1);
                assert_eq!(tf_status[0].view, view);
            }
            _ => unreachable!(),
        }

        app.tf_engine.execute("/set status_height=2");
        app.tf_status_at = None;
        app.refresh_tf_status();
        assert_eq!(app.separator_rows(), 2);
    }

    /// Mail checking, once a script asks for it: mail there at the first check is
    /// mentioned, new mail after that runs the MAIL hook after TF's message, and nmail()
    /// counts the files with unread mail.
    #[test]
    fn test_mail_check() {
        use std::time::{Duration, SystemTime};
        let mut app = app_with_world();
        let dir = std::env::temp_dir().join(format!("clay_mail_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mailbox = dir.join("inbox");
        let write_mail = |unread: bool| {
            std::fs::write(&mailbox, "From someone\n").unwrap();
            let base = SystemTime::now() - Duration::from_secs(100);
            let (read, written) = if unread { (base, base + Duration::from_secs(10)) } else { (base + Duration::from_secs(10), base) };
            let file = std::fs::File::options().write(true).open(&mailbox).unwrap();
            file.set_times(std::fs::FileTimes::new().set_accessed(read).set_modified(written)).unwrap();
        };
        write_mail(false);
        app.check_tf_mail();
        assert!(app.tf_mail_files.is_empty(), "not asked for: nothing checked");

        app.tf_engine.execute(&format!("/set TFMAILPATH={}", mailbox.display()));
        app.tf_engine.execute("/def -hMAIL onmail = /echo hooked %*");
        app.tf_mail_due = None;
        app.check_tf_mail();
        assert_eq!(app.tf_engine.mail_count, 0);
        write_mail(true);
        app.tf_mail_due = None;
        app.check_tf_mail();
        assert_eq!(app.tf_engine.mail_count, 1);
        let t = texts(&app);
        let new = t.iter().position(|l| l == &format!("% You have new mail in {}.", mailbox.display())).expect("TF's message");
        let hooked = t.iter().position(|l| l.starts_with("hooked ")).expect("the MAIL hook");
        assert!(new < hooked, "{:?}", t);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A trigger's `A` attribute keeps its line from counting as activity on a background
    /// world (TF's noactivity); its `L` marks the line not to be logged.
    #[test]
    fn test_noactivity_attribute() {
        let mut app = app_with_world();
        let mut other = crate::World::new("Other");
        other.showing_splash = false;
        app.worlds.push(other);
        app.current_world_index = 0;
        app.tf_engine.execute("/def -aA -t\"chatter*\" quiet");
        app.process_server_data(1, b"chatter one\r\nreal news\r\nchatter two\r\n", 24, 80, false);
        assert_eq!(app.worlds[1].unseen_lines, 1, "only the line without A is activity");
        assert_eq!(app.worlds[1].output_lines.len(), 3, "A lines are still shown");
        assert!(app.worlds[1].tf_noactivity.is_empty() && app.worlds[1].tf_nolog.is_empty());
    }

    /// Clay's own status bar keeps its 12-hour clock until a script sets %clock_format;
    /// then it reads that way, and clients are told.
    #[test]
    fn test_clock_format_reaches_clays_bar() {
        let mut app = app_with_world();
        app.apply_tf_clock_format(None);
        assert_eq!(app.settings.clock_format, "", "TF's default alone doesn't change Clay's clock");
        // As typed (or in an rc): not %-expanded.
        let effects = app.tf_engine.run_typed("/set clock_format=%H:%M:%S");
        app.apply_tf_effects_now(effects, TfEffectCtx::for_world(0, false));
        assert_eq!(app.settings.clock_format, "%H:%M:%S");
        let told = app.ws_broadcast_log.lock().unwrap().iter().any(|m| matches!(m,
            crate::WsMessage::GlobalSettingsUpdated { settings, .. } if settings.clock_format == "%H:%M:%S"));
        assert!(told);
        assert_eq!(crate::util::format_clock("%H:%M:%S").len(), 8);
    }

    /// %wrapsize, once a script sets it, is where output wraps (unless %wrap is off) - in
    /// the console's own rows, and clients are told; unset, the window's edge.
    #[test]
    fn test_wrapsize_caps_output_width() {
        let mut app = app_with_world();
        assert_eq!(app.settings.wrap_columns, 0);
        let effects = app.tf_engine.run("/wrap 20");
        app.apply_tf_effects_now(effects, TfEffectCtx::for_world(0, false));
        assert_eq!(app.settings.wrap_columns, 20);
        let line = crate::OutputLine::new("word ".repeat(10).trim_end().to_string(), 1);
        let rows = crate::rendering::display_wrapped(&line, 80, false, &app.settings, &crate::CachedNow::new());
        assert!(rows.len() > 1 && rows.iter().all(|r| crate::tf::status::text_columns(r) <= 20), "{:?}", rows);
        let told = app.ws_broadcast_log.lock().unwrap().iter().any(|m| matches!(m,
            crate::WsMessage::GlobalSettingsUpdated { settings, .. } if settings.wrap_columns == 20));
        assert!(told, "clients cap their output too");
        let effects = app.tf_engine.run("/set wrap=off");
        app.apply_tf_effects_now(effects, TfEffectCtx::for_world(0, false));
        assert_eq!(app.settings.wrap_columns, 0, "wrap off: the window's edge");
    }

    /// A bound variable's new value survives a re-sync of Clay's settings that happens
    /// before its effect is applied - an /addworld earlier in the same rc re-syncs them,
    /// which used to set a variable later in the rc back to Clay's value.
    #[test]
    fn test_bound_setting_survives_resync() {
        let mut app = app_with_world();
        let effects = app.tf_engine.run("/set isize=5");
        app.sync_tf_world_info();
        app.apply_tf_effects_now(effects, TfEffectCtx::for_world(0, false));
        assert_eq!(app.input_height, 5);
        let effects = app.tf_engine.run("/set more=on");
        app.sync_tf_world_info();
        app.apply_tf_effects_now(effects, TfEffectCtx::for_world(0, false));
        assert!(app.settings.more_mode_enabled);
    }

    /// columns(), lines(), winlines(), moresize(), morepaused() and nlog() report the
    /// App's own state, synced with the world info.
    #[test]
    fn test_screen_and_more_functions_read_app_state() {
        let mut app = app_with_world();
        app.screen_size = Some((132, 50));
        app.output_height = 45;
        app.worlds[0].paused = true;
        app.worlds[0].pending_lines = vec![crate::OutputLine::new("a".to_string(), 1), crate::OutputLine::new("b".to_string(), 2)];
        let log = std::env::temp_dir().join(format!("clay_nlog_{}", std::process::id()));
        app.worlds[0].log_handle = Some(std::sync::Arc::new(std::sync::Mutex::new(std::fs::File::create(&log).unwrap())));
        app.sync_tf_world_info();
        let world = app.worlds[0].name.clone();
        let mut eval = |expr: &str| crate::tf::expressions::evaluate(&mut app.tf_engine, expr).unwrap().to_string_value();
        assert_eq!(eval("columns()"), "132");
        assert_eq!(eval("lines()"), "50");
        assert_eq!(eval("winlines()"), "45");
        assert_eq!(eval(&format!("moresize(\"\", \"{}\")", world)), "2");
        assert_eq!(eval(&format!("moresize(\"ln\", \"{}\")", world)), "2");
        assert_eq!(eval(&format!("morepaused(\"{}\")", world)), "1");
        assert_eq!(eval("moresize(\"\", \"nosuch\")"), "0");
        assert_eq!(eval("nlog()"), "1");
        let _ = std::fs::remove_file(&log);
    }

    /// TF's catch-prompt idiom: a PROMPT hook takes the prompt over and shows its own
    /// with prompt(); /prompt -aB sets one in bold. Both reach the world's prompt.
    #[test]
    fn test_prompt_hook_and_prompt_command() {
        let mut app = app_with_world();
        app.worlds[0].connected = true;
        app.tf_engine.execute("/def -h\"PROMPT *>\" catch = /test prompt(strcat(\"[\", {*}, \"]\"))");
        app.handle_prompt_text(0, "HP:10> ", crate::PromptSource::Marker);
        assert_eq!(app.worlds[0].prompt, "[HP:10>]");

        app.sync_tf_world_info();
        let effects = app.run_tf_in_world(0, "/prompt -aB ready");
        app.apply_tf_effects_now(effects, TfEffectCtx::for_world(0, false));
        assert_eq!(app.worlds[0].prompt, "\x1b[1mready\x1b[0m");
        assert_eq!(app.tf_engine.get_var("?").map(|v| v.to_string_value()).as_deref(), Some("1"));
    }

    /// /sh on the console gets the terminal (queued for the console loop); from a client
    /// it runs in the background, its output and status coming back as TfShellDone, and
    /// a bare /sh - an interactive shell - can't be had there.
    #[tokio::test]
    async fn test_sh_console_gets_terminal_client_runs_captured() {
        let mut app = app_with_world();
        app.console_tui = true;
        let (event_tx, mut event_rx) = mpsc::channel::<AppEvent>(16);
        app.tf_event_tx = Some(event_tx.clone());

        let effects = app.tf_engine.run_typed("/sh -q echo typed");
        app.apply_tf_effects_now(effects, TfEffectCtx::typed(0));
        assert_eq!(app.pending_terminal_ops.pop_front(),
            Some(crate::TerminalOp::Shell { command: Some("echo typed".to_string()), world_idx: 0 }));

        let _ = app.handle_ws_send_command_impl(1, 0, "/sh -q echo from client; exit 4", &event_tx);
        assert!(app.pending_terminal_ops.is_empty(), "a client never gets this machine's terminal");
        let done = tokio::time::timeout(std::time::Duration::from_secs(10), event_rx.recv()).await;
        match done {
            Ok(Some(AppEvent::TfShellDone(world_idx, output, status))) => {
                assert_eq!((world_idx, output.as_str(), status), (0, "from client\n", 4));
                app.finish_tf_shell(world_idx, &output, status, false);
            }
            _ => panic!("expected the shell's result"),
        }
        assert!(texts(&app).iter().any(|t| t == "from client"), "{:?}", texts(&app));
        assert_eq!(app.tf_engine.get_var("?").map(|v| v.to_string_value()).as_deref(), Some("4"));

        let _ = app.handle_ws_send_command_impl(1, 0, "/sh", &event_tx);
        assert!(texts(&app).iter().any(|t| t.contains("interactive shell needs Clay's own console")), "{:?}", texts(&app));
        let _ = app.handle_ws_send_command_impl(1, 0, "/suspend", &event_tx);
        assert!(app.pending_terminal_ops.is_empty());
    }

    /// A client's /help on a topic only TinyFugue's help file has gets that help, to that
    /// client alone: several words, original case, and `%kecho` looked up, not expanded.
    #[test]
    fn test_client_help_reads_tf_help_file() {
        let mut app = app_with_world();
        let dir = std::env::temp_dir().join(format!("clay_tfrun_help_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("tf-help");
        std::fs::write(&file, "&special variables\n\nall of them\n#kecho\n#%kecho\n  kecho=off\n").unwrap();
        app.tf_engine.set_global("TFHELP", crate::tf::TfValue::String(file.display().to_string()));
        let (gui_tx, mut gui_rx) = mpsc::unbounded_channel::<crate::WsMessage>();
        app.gui_tx = Some(gui_tx);
        let (event_tx, _event_rx) = mpsc::channel::<AppEvent>(16);
        let mut answer = |app: &mut App, line: &str| {
            let _ = app.handle_ws_send_command_impl(0, 0, line, &event_tx);
            let mut data = String::new();
            while let Ok(msg) = gui_rx.try_recv() {
                if let crate::WsMessage::ServerData { data: d, .. } = msg {
                    data.push_str(&d);
                }
            }
            data
        };
        assert!(answer(&mut app, "/help special variables").starts_with("Help on: special variables\n\nall of them"));
        assert!(answer(&mut app, "/help %kecho").starts_with("Help on: special variables: %kecho\n  kecho=off"));
        assert_eq!(answer(&mut app, "/help Nosuch Topic"), "% Help on subject Nosuch Topic not found.\n");
        assert!(texts(&app).iter().all(|t| !t.contains("Help on")), "help is the asking client's alone");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Loading a file, an error and an unknown command each say where they are in it,
    /// TF-style; the same error typed keeps Clay's "Error:" form.
    #[tokio::test]
    async fn test_load_errors_and_unknown_commands_name_their_line() {
        let mut app = app_with_world();
        let (event_tx, _event_rx) = mpsc::channel::<AppEvent>(16);
        let dir = std::env::temp_dir().join(format!("clay_tfrun_load_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("rc.tf");
        std::fs::write(&file, "/set more=foo\n/zzz_nothing x\n/echo done\n").unwrap();
        let _ = run_tf_line_console(&mut app, &format!("/load {}", file.display()), &event_tx).await;
        let _ = run_tf_line_console(&mut app, "/set more=bar", &event_tx).await;
        let _ = std::fs::remove_dir_all(&dir);
        let path = file.display().to_string();
        let t = texts(&app);
        let has = |needle: &str| t.iter().any(|l| l == needle);
        assert!(has(&format!("% {}, line 1: Invalid more value \"foo\".  Valid values are: off (0), on (1)", path)), "{t:?}");
        assert!(has(&format!("% {}, line 2: Unknown command: /zzz_nothing", path)), "{t:?}");
        assert!(has("done"), "{t:?}");
        assert!(has("Error: Invalid more value \"bar\".  Valid values are: off (0), on (1)"), "{t:?}");
    }
}
