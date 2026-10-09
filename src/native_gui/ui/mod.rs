//! The GPUI side of the native GUI.

mod actions;
pub mod assets;
mod bars;
mod chrome;
mod dialogs;
pub mod emoji;
mod find;
mod follow;
pub mod fonts;
pub mod ink;
mod input_view;
mod keymap;
mod kit;
mod native_spell;
mod more;
pub mod output;
mod select;
mod surfaces;
mod textdiv;
mod theme_bridge;
mod viewmode;
mod windows;
#[cfg(feature = "native-gui-snapshot")]
pub mod snapshot;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{
    App, AppContext, Bounds, Context, Entity, FocusHandle, Focusable, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, ParentElement, Render, ScrollWheelEvent, Styled, TitlebarOptions, Window, WindowBounds, WindowOptions, div,
    prelude::*, px, size,
};
use gpui::{KeyBinding, Keystroke, SharedString};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::input::{Input, InputEvent, InputState};

use crate::native_gui::core::client::{Credentials, Effect, Input as NetInput, Status, hidden_line};
use crate::native_gui::core::keys::{Keys, Resolved};
use crate::native_gui::core::session::Session;
use crate::native_gui::core::decor::{ActionMatcher, DecorOptions, LineFacts, decorate};
use crate::native_gui::core::style::{Palette, StyleOptions, StyledLine};
use crate::theme::ThemeColors;
use chrome::{Chrome, Tokens};
use fonts::Fonts;
use more::MoreMode;
use output::{OutputSnapshot, ScrollHandle, hsla, output_view};

/// `web_font_size_desktop`'s default.
const DEFAULT_FONT_SIZE: f32 = 18.0;

/// When `ui::run` began, for the first frame's startup time in the debug log.
static STARTED: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

/// When to write the world cache.
#[derive(Default)]
struct CacheSaver {
    /// The cache revision last looked at.
    seen: u64,
    /// The last change, and the first one not yet saved.
    changed: Option<Instant>,
    dirty_since: Option<Instant>,
}

/// The fields of the login screen.
struct LoginForm {
    username: Entity<InputState>,
    password: Entity<InputState>,
}

pub struct ClayView {
    focus: FocusHandle,
    fonts: Fonts,
    session: Session,
    theme: ThemeColors,
    theme_json: String,
    palette: Palette,
    style: StyleOptions,
    show_tags: bool,
    font_size: f32,
    wrapspace: usize,
    wrap_columns: usize,
    letter_spacing: f32,
    word_spacing: f32,
    /// Decorated lines by `Line::id` (with whether they carried ▶), valid while the
    /// style inputs are unchanged.
    styled: HashMap<u64, (bool, Arc<StyledLine>)>,
    decor: DecorOptions,
    matcher: ActionMatcher,
    /// What `matcher` was built from.
    actions_sig: String,
    /// Bumped whenever the same line would style differently (clears `styled`).
    style_epoch: u64,
    scroll: ScrollHandle,
    more: MoreMode,
    keys: Keys,
    /// TF's numeric prefix (`%kbnum`), shown as [N] after the world name.
    kbnum: Option<i64>,
    kbnum_sign: i64,
    /// F8: lines matching an enabled action get a highlight.
    action_highlight: bool,
    /// The world the output last showed (a switch resets more-mode and the scroll).
    shown_world: Option<usize>,
    login: LoginForm,
    /// The login screen has put the cursor in its first empty field.
    login_focused: bool,
    chrome: Chrome,
    select: select::SelectState,
    textdiv: textdiv::Textdiv,
    find: find::FindState,
    /// The input line's text and editing state.
    editor: crate::native_gui::core::input::Editor,
    input: input_view::InputState,
    /// The `input_height` last taken from the server (input_grow/shrink change the rows
    /// locally until it changes, as app.js does).
    input_height_seen: u16,
    /// The theme gpui-component's widgets were last given.
    bridged_theme: String,
    /// The disk cache (the real window only).
    cache: Option<CacheSaver>,
    /// Dialogs, and what they wait on.
    surf: surfaces::Surfaces,
    /// The other windows.
    windows: windows::Windows,
    /// What this window of output is for (the main window, `/window`, `--grep`).
    mode: viewmode::ViewMode,
    /// The windows sharing this one's connection, or the window whose connection this
    /// one shares.
    link: follow::Link,
    /// The MUD's sounds and the server's speech: the main window's (the others get the
    /// same messages, and mustn't play them again).
    media: Option<crate::native_gui::core::media::Media>,
    /// The current world and whether its media is on, as last seen: turning it off
    /// stops what it started (app.js `GmcpUserToggled`).
    media_gate: Option<(usize, bool)>,
}

impl ClayView {
    fn new(fonts: Fonts, session: Session, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let username = cx.new(|cx| InputState::new(window, cx).placeholder("User name"));
        let password = cx.new(|cx| InputState::new(window, cx).masked(true).placeholder("Password"));
        for input in [&username, &password] {
            cx.subscribe_in(input, window, |this, _, ev: &InputEvent, _window, cx| {
                if let InputEvent::PressEnter { .. } = ev {
                    this.submit_login(cx);
                }
            })
            .detach();
        }
        // Minimised, fully covered (macOS) or on a sleeping display: not visible, so text
        // arriving meanwhile is unviewed and gets its ▶ when the window comes back (app.js
        // does the same on `visibilitychange`).
        cx.observe_window_visibility(window, |this, vis, _window, cx| {
            let fx = this.session.core.set_visible(vis.is_visible(), Instant::now());
            let rest = this.session.apply(fx);
            this.after(rest, cx);
        })
        .detach();
        let mut session = session;
        let _ = session.core.set_visible(window.is_visible(), Instant::now());
        let theme = ThemeColors::dark_default();
        let tokens = Tokens::new(&theme, false);
        let mut view = ClayView {
            focus: cx.focus_handle(),
            fonts,
            session,
            palette: Palette::from_theme(&theme),
            theme,
            theme_json: String::new(),
            style: StyleOptions::default(),
            show_tags: false,
            font_size: DEFAULT_FONT_SIZE,
            wrapspace: 0,
            wrap_columns: 0,
            letter_spacing: 0.0,
            word_spacing: 0.0,
            styled: HashMap::new(),
            decor: DecorOptions { show_tags: false, new_markers: false, temp_convert: false, accent: [0, 0, 0], hover_bg: [0, 0, 0] },
            matcher: ActionMatcher::default(),
            actions_sig: String::new(),
            style_epoch: 0,
            scroll: ScrollHandle::default(),
            more: MoreMode::default(),
            keys: Keys::default(),
            kbnum: None,
            kbnum_sign: 1,
            action_highlight: false,
            shown_world: None,
            login: LoginForm { username, password },
            login_focused: false,
            chrome: Chrome::new(tokens),
            select: select::SelectState::default(),
            textdiv: textdiv::Textdiv::default(),
            find: find::FindState::new(window, cx),
            editor: Default::default(),
            input: Default::default(),
            input_height_seen: 0,
            bridged_theme: String::new(),
            cache: None,
            surf: Default::default(),
            windows: Default::default(),
            mode: Default::default(),
            link: Default::default(),
            media: None,
            media_gate: None,
        };
        view.apply_settings();
        theme_bridge::apply(&view.theme, &view.fonts, cx);
        view.bridged_theme = view.theme_json.clone();
        view
    }

    /// Effects the session left for us: redraw, and the GUI's own messages.
    fn after(&mut self, effects: Vec<Effect>, cx: &mut Context<Self>) {
        let mut changed = false;
        let mut local = Vec::new();
        for e in effects {
            match e {
                Effect::Changed => changed = true,
                Effect::Event(msg) => match *msg {
                    crate::websocket::WsMessage::PendingReleased { world_index, count } => {
                        // Released by any interface: count afresh, and show as many of ours.
                        self.more.caught_up();
                        if world_index == self.session.core.current {
                            self.more.release(count);
                        }
                        changed = true;
                    }
                    ref m if self.surface_message(m, cx) => {}
                    crate::websocket::WsMessage::ExecuteLocalCommand { command } => local.push(command),
                    crate::websocket::WsMessage::McmpMedia { world_index, action, data, default_url } => {
                        // The world's own media plays only while it's shown and switched on;
                        // a Default is kept whatever (app.js).
                        let core = &self.session.core;
                        let on = core.worlds.get(world_index).is_some_and(|w| w.gmcp_user_enabled) && world_index == core.current;
                        if let Some(m) = self.media.as_mut().filter(|_| action == "Default" || on) {
                            m.mcmp(&action, &data, &default_url);
                        }
                    }
                    crate::websocket::WsMessage::AnsiMusic { notes, .. } => {
                        let enabled = self.session.core.settings.as_ref().is_some_and(|s| s.ansi_music_enabled);
                        if let Some(m) = self.media.as_mut().filter(|_| enabled && !notes.is_empty()) {
                            m.ansi_music(&notes);
                        }
                    }
                    crate::websocket::WsMessage::ServerSpeak { text, .. } => {
                        let mode = crate::tts::TtsMode::from_name(self.session.core.settings.as_ref().map_or("", |s| s.tts_mode.as_str()));
                        if let Some(m) = self.media.as_mut() {
                            m.speak(&text, mode);
                        }
                    }
                    // The web client passes these to Android's notifications only.
                    crate::websocket::WsMessage::Notification { .. } => {}
                    // The server puts text in the input (TF's /input and the like).
                    crate::websocket::WsMessage::SetInputBuffer { text, cursor_start } => {
                        self.editor.clear();
                        self.editor.insert(&text);
                        if cursor_start {
                            self.editor.home();
                        }
                        changed = true;
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        for command in local {
            self.local_command(&command, cx);
        }
        self.watch_status(cx);
        self.keep_to_world(cx);
        self.watch_media();
        if changed {
            self.apply_settings();
            if std::mem::take(&mut self.surf.actions_changed) {
                cx.emit(windows::MainEvent::Actions);
            }
            self.watch_worlds(cx);
            self.watch_settings(cx);
            if self.bridged_theme != self.theme_json {
                theme_bridge::apply(&self.theme, &self.fonts, cx);
                self.bridged_theme = self.theme_json.clone();
                self.restyled(cx);
            }
            self.follow_world();
            cx.notify();
        }
        self.relay_outbox(cx);
    }

    /// The current world's media switched off (F9, the MUD's toggle): stop what it started.
    fn watch_media(&mut self) {
        let Some(m) = self.media.as_mut() else { return };
        let core = &self.session.core;
        let now = core.current_world().map(|w| (core.current, w.gmcp_user_enabled));
        if let (Some((was_world, true)), Some((world, false))) = (self.media_gate, now) {
            if was_world == world {
                m.stop_all();
            }
        }
        self.media_gate = now;
    }

    /// After any change: a world switch starts the output over; new lines at the tail go
    /// through more-mode.
    fn follow_world(&mut self) {
        let current = self.session.core.current_world().map(|_| self.session.core.current);
        self.textdiv.note_worlds(&self.session.core.worlds);
        if current != self.shown_world {
            if let Some(old) = self.shown_world.and_then(|i| self.session.core.worlds.get(i)) {
                self.textdiv.leave(old);
            }
            if let Some(new) = self.session.core.current_world() {
                let mode = self.session.core.settings.as_ref().map_or("", |s| s.textdiv.as_str());
                self.textdiv.arrive(new, mode);
                // Remembered, to come back to next time (app.js persistLastActiveWorld).
                let name = new.name.clone();
                if !name.is_empty() && !self.is_follower() {
                    crate::native_gui::core::store::set_last_world(&self.session.target.key(), &name);
                    self.session.core.prefer_world(Some(name));
                }
            }
            self.shown_world = current;
            // A switch stops the MUD's media; the server sends the new world's (app.js).
            if let Some(m) = self.media.as_mut() {
                m.stop_all();
            }
            self.more.reset(self.session.core.current_world());
            self.scroll.borrow_mut().follow_bottom();
            return;
        }
        let threshold = self.scroll.borrow().view_rows.saturating_sub(2);
        let enabled = self.more_mode_enabled();
        if let Some(world) = self.session.core.current_world() {
            if self.more.arrivals(world, enabled, threshold) {
                self.scroll.borrow_mut().follow_bottom();
            }
        }
        // Back at the bottom with nothing held: output flows again.
        if self.scroll.borrow().at_bottom() && self.pending_total() == 0 {
            self.more.paused = false;
        }
    }

    fn more_mode_enabled(&self) -> bool {
        self.session.core.settings.as_ref().is_some_and(|s| s.more_mode_enabled)
    }

    /// Lines will style differently from now on.
    fn restyle(&mut self) {
        self.style_epoch += 1;
        self.styled.clear();
    }

    /// A keystroke, resolved as every Clay client resolves it.
    fn key_down(&mut self, ks: &gpui::Keystroke, window: &mut Window, cx: &mut Context<Self>) -> bool {
        // Any key closes the right-click menu (as the GUI's does); Escape closes the
        // Debug Selection dialog.
        if self.select.menu.take().is_some() | self.input.menu.take().is_some() {
            cx.notify();
        }
        if self.select.debug.is_some() && ks.key == "escape" {
            self.select.debug = None;
            cx.notify();
            return true;
        }
        // While the input method is composing, it has the keys (Enter commits, arrows move
        // within the composition).
        if self.editor.marked.is_some() {
            return false;
        }
        // ^V: the next keystroke goes in as itself (TF's literal-next).
        if self.input.literal_next {
            if let Some(c) = keymap::console_key(ks).and_then(|(code, mods)| crate::input::literal_char(code, mods)) {
                self.input.literal_next = false;
                self.editor.insert(&c.to_string());
                self.typed(cx);
                return true;
            }
            return false; // a bare modifier: still waiting
        }
        // The clipboard. Copy takes the selection, in the input or the output; paste is
        // Ctrl+Shift+V, Shift+Insert or Cmd+V (Ctrl+V is literal-next by the key table);
        // Cmd+X and Cmd+A on a Mac (^X and ^A are Clay's).
        let m = &ks.modifiers;
        let key = ks.key.as_str();
        if key == "c" && (m.platform || (m.control && !m.alt && !m.shift)) && (self.copy_input(false, cx) || self.copy_selection(cx)) {
            return true;
        }
        if (key == "v" && (m.platform || (m.control && m.shift))) || (key == "insert" && m.shift && !m.control) {
            self.paste(cx);
            return true;
        }
        if m.platform && key == "x" {
            return self.copy_input(true, cx);
        }
        if m.platform && key == "a" {
            self.editor.select_all();
            self.input_touched(cx);
            return true;
        }
        let Some((code, mods)) = keymap::console_key(ks) else { return false };
        let resolved = self.keys.resolve(code, mods, Instant::now());
        // A history search ends at any key but its own.
        match &resolved {
            Resolved::Pending => {}
            Resolved::Action { key, .. } | Resolved::Server(key) if key == "Esc-p" || key == "Esc-n" => {}
            _ => self.editor.end_search(),
        }
        match resolved {
            Resolved::Pending => true,
            Resolved::Server(key) => {
                let kbnum = self.kbnum.take();
                self.kbnum_sign = 1;
                let fx = self.session.send(crate::websocket::WsMessage::RunKeyBinding { key, kbnum });
                self.after(fx, cx);
                true
            }
            Resolved::Action { action, .. } => {
                self.run_action(&action, window, cx);
                true
            }
            Resolved::NotBound => {
                // Enter sends, bound or not (by the raw key, as app.js and the console do).
                if key == "enter" && !m.control && !m.alt {
                    self.send_line(window, cx);
                    return true;
                }
                // Unbound Shift+arrows extend the selection.
                if m.shift && !m.control && !m.alt && matches!(key, "left" | "right" | "home" | "end") {
                    let e = &mut self.editor;
                    let to = match key {
                        "home" => 0,
                        "end" => e.text.len(),
                        "left" => e.text[..e.cursor].char_indices().last().map_or(0, |(i, _)| i),
                        _ => e.text[e.cursor..].chars().next().map_or(e.cursor, |c| e.cursor + c.len_utf8()),
                    };
                    e.set_cursor(to, true);
                    self.input_touched(cx);
                    return true;
                }
                // %insert off: a typed character replaces the one under the cursor.
                if self.editor.overwrite && !m.control && !m.alt && !m.platform {
                    if let Some(ch) = ks.key_char.as_deref().filter(|c| c.chars().count() == 1 && !c.starts_with(char::is_control)) {
                        self.editor.overwrite(ch);
                        self.typed(cx);
                        return true;
                    }
                }
                // Typed text arrives through the input handler.
                false
            }
        }
    }

    fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let fx = self.session.tick();
        self.after(fx, cx);
        if let Some(m) = self.media.as_mut() {
            m.poll();
        }
        // A chord left waiting fires its own binding.
        match self.keys.tick(Instant::now()) {
            Some(Resolved::Action { action, .. }) => self.run_action(&action, window, cx),
            Some(Resolved::Server(key)) => {
                let fx = self.session.send(crate::websocket::WsMessage::RunKeyBinding { key, kbnum: self.kbnum.take() });
                self.after(fx, cx);
            }
            _ => {}
        }
        if matches!(self.session.core.status(), Status::Reconnecting { .. }) {
            cx.notify(); // the countdown
        }
        self.drag_scroll(cx);
        self.blink(cx);
        self.save_cache();
        let clock = chrome::clock_text(self.session.core.settings.as_ref().map_or("", |s| s.clock_format.as_str()));
        if clock != self.chrome.clock {
            self.chrome.clock = clock;
            cx.notify();
        }
    }

    /// Write the world cache once output has been quiet for two seconds (app.js's
    /// debounce), or every 30 seconds while it never is.
    fn save_cache(&mut self) {
        let Some(c) = self.cache.as_mut() else { return };
        let now = Instant::now();
        let rev = self.session.core.cache_revision();
        if rev != c.seen {
            c.seen = rev;
            c.changed = Some(now);
            c.dirty_since.get_or_insert(now);
        }
        let (Some(changed), Some(since)) = (c.changed, c.dirty_since) else { return };
        if now - changed < Duration::from_secs(2) && now - since < Duration::from_secs(30) {
            return;
        }
        c.changed = None;
        c.dirty_since = None;
        let worlds = self.session.core.cache_snapshot();
        if !worlds.is_empty() {
            let key = self.session.target.key();
            std::thread::spawn(move || crate::native_gui::core::cache::save(&key, &worlds));
        }
    }

    fn submit_login(&mut self, cx: &mut Context<Self>) {
        let username = self.login.username.read(cx).value().to_string();
        let password = self.login.password.read(cx).value().to_string();
        if password.is_empty() {
            return;
        }
        let creds = Credentials { username: (!username.is_empty()).then_some(username), password: Some(password), auth_key: None };
        let fx = self.session.login(creds);
        self.after(fx, cx);
    }

    /// The server's display settings, whenever they change.
    fn apply_settings(&mut self) {
        let Some(s) = self.surf.preview.as_ref().or(self.session.core.settings.as_ref()) else { return };
        self.keys.update(s);
        let mut restyle = false;
        if !s.theme_colors_json.is_empty() && s.theme_colors_json != self.theme_json {
            self.theme_json = s.theme_colors_json.clone();
            self.theme = ThemeColors::from_json(&s.theme_colors_json, &ThemeColors::dark_default());
            self.palette = Palette::from_theme(&self.theme);
            restyle = true;
        }
        if s.web_font_size_desktop > 0.0 {
            self.font_size = s.web_font_size_desktop;
        }
        // Font and wrapping settings change every line's layout.
        let layout = (s.wrapspace as usize, s.wrap_columns, s.web_font_letter_spacing, s.web_font_word_spacing);
        if layout != (self.wrapspace, self.wrap_columns, self.letter_spacing, self.word_spacing) {
            (self.wrapspace, self.wrap_columns, self.letter_spacing, self.word_spacing) = layout;
            restyle = true;
        }
        restyle |= self.fonts.set_user(&s.font_name, s.web_font_weight);
        let (spell_on, dict) = (s.spell_check_enabled, s.dictionary_path.clone());
        if s.input_height != self.input_height_seen {
            self.input_height_seen = s.input_height;
            self.input.rows = (s.input_height as usize).clamp(1, 15);
        }
        self.spell_settings(spell_on, &dict);
        let Some(s) = self.surf.preview.as_ref().or(self.session.core.settings.as_ref()) else { return };
        let tokens = Tokens::new(&self.theme, s.gui_theme == "light");
        if tokens != self.chrome.tokens {
            self.chrome.tokens = tokens;
        }
        self.chrome.clock = chrome::clock_text(&s.clock_format);
        let offset = s.color_offset_percent as u32;
        if offset != self.style.color_offset_percent || s.show_tags != self.show_tags {
            self.style.color_offset_percent = offset;
            self.show_tags = s.show_tags;
            self.style.discord_emoji = !s.show_tags;
            restyle = true;
        }
        let rgb = |c: &crate::theme::ThemeColor| [c.r, c.g, c.b];
        let decor = DecorOptions {
            show_tags: s.show_tags,
            new_markers: s.new_line_indicator && s.textdiv.is_empty(),
            temp_convert: s.temp_convert_enabled,
            accent: rgb(&self.theme.accent),
            hover_bg: rgb(&self.theme.bg_hover),
        };
        if decor != self.decor {
            self.decor = decor;
            restyle = true;
        }
        let sig = serde_json::to_string(&self.session.core.actions).unwrap_or_default();
        if sig != self.actions_sig {
            self.matcher = ActionMatcher::new(&self.session.core.actions);
            self.actions_sig = sig;
            self.surf.actions_changed = true;
            restyle |= self.action_highlight;
        }
        if restyle {
            self.restyle();
        }
    }

    /// The current world's lines as the output shows them, with their ids.
    fn visible_lines(&mut self) -> Vec<(u64, Arc<StyledLine>)> {
        let Some(world) = self.session.core.current_world() else {
            // No world yet (multiuser, before connecting): the ASCII splash.
            let lines: Vec<(u64, Arc<StyledLine>)> = self
                .session
                .core
                .splash_lines
                .iter()
                .enumerate()
                .map(|(k, l)| (textdiv::DIVIDER_ID / 2 + k as u64, Arc::new(crate::native_gui::core::style::style_line(l, &self.palette, &self.style))))
                .collect();
            self.note_visible(lines.iter().map(|(id, _)| *id).collect());
            return lines;
        };
        let me = self.session.core.display_id;
        let mut out = Vec::with_capacity(world.lines.len());
        let mut seqs = Vec::with_capacity(world.lines.len());
        let filter = self.find.filtering().map(str::to_string);
        let cut = self.find.search_cut();
        for line in &world.lines {
            if (line.gagged && !self.show_tags) || hidden_line(&line.text) || self.more.is_held(line.id) {
                if Some(line.id) == cut {
                    break;
                }
                continue;
            }
            // F4: only the lines containing Find's text.
            if filter.as_deref().is_some_and(|f| !find::matches_filter(&find::plain(&line.text), f)) {
                continue;
            }
            // A grep window: only the lines matching its pattern.
            if self.mode.grep.as_ref().is_some_and(|g| !g.matches(&line.text)) {
                continue;
            }
            seqs.push(line.seq.filter(|_| !line.from_archive));
            let is_new = me != 0 && line.display_id == Some(me);
            if let Some((was_new, styled)) = self.styled.get(&line.id) {
                if *was_new == is_new {
                    out.push((line.id, styled.clone()));
                    if Some(line.id) == cut {
                        break;
                    }
                    continue;
                }
            }
            let facts = LineFacts {
                text: &line.text,
                ts: line.ts,
                from_server: line.from_server,
                from_archive: line.from_archive,
                archive_sourced: line.archive_sourced,
                is_new,
                highlight: line.highlight.as_deref(),
                action_match: self.action_highlight && self.matcher.matches(&line.text, &world.name),
            };
            let styled = Arc::new(decorate(&facts, &self.palette, &self.style, &self.decor));
            self.styled.insert(line.id, (is_new, styled.clone()));
            out.push((line.id, styled));
            if Some(line.id) == cut {
                // F5: the output ends at the match.
                break;
            }
        }
        if self.styled.len() > 2 * out.len() + 1024 {
            let live: std::collections::HashSet<u64> = world.lines.iter().map(|l| l.id).collect();
            self.styled.retain(|id, _| live.contains(id));
        }
        // TF's %textdiv divider for this visit (not in a filtered view).
        if let Some(at) = self.textdiv.place(&seqs).filter(|_| filter.is_none()) {
            let settings = self.session.core.settings.as_ref();
            let rows: Vec<(u64, Arc<StyledLine>)> = match self.textdiv.visit.as_ref().map(|v| v.1.as_str()) {
                Some("clear") => {
                    let n = self.scroll.borrow().view_rows.max(1) as u64;
                    (0..n).map(|k| (textdiv::DIVIDER_ID + k, Arc::new(StyledLine::default()))).collect()
                }
                _ => {
                    let text = settings.map_or("=====", |s| s.textdiv_str.as_str());
                    vec![(textdiv::DIVIDER_ID, Arc::new(crate::native_gui::core::style::style_line(text, &self.palette, &self.style)))]
                }
            };
            out.splice(at..at, rows);
        }
        self.note_visible(out.iter().map(|(id, _)| *id).collect());
        out
    }

    /// The connection's state, while it isn't simply online (the status bar shows it
    /// beside the world).
    fn connection_note(&self) -> Option<String> {
        Some(match self.session.core.status() {
            Status::Online => return None,
            Status::Idle | Status::Connecting { .. } => format!("Connecting to {}…", self.session.target.key()),
            Status::LoggingIn => "Logging in…".into(),
            Status::NeedLogin { .. } => "Log in".into(),
            Status::CertMismatch(_) => "The server's certificate has changed".into(),
            Status::Reconnecting { at, reason, .. } => {
                let secs = at.saturating_duration_since(Instant::now()).as_secs_f32().ceil() as u64;
                format!("Reconnecting in {secs} s ({reason})")
            }
            Status::Stopped(why) => why.clone(),
        })
    }

    /// Tell the server how much the output shows (it sizes more-mode and history by it).
    fn report_view(&mut self, window: &Window) {
        let vp = window.viewport_size();
        // The rows the output showed when last laid out (the bars around it take the rest).
        let rows = self.scroll.borrow().view_rows.max(1);
        let cols = ((f32::from(vp.width) - 8.0) / (self.font_size * 0.6)).floor().max(1.0) as usize;
        let fx = self.session.core.set_view(rows, cols);
        let _ = self.session.apply(fx);
    }

    /// The GUI's splash (app.js `renderOutput` in WEBVIEW_MODE): the logo and two lines,
    /// centred, in the output's font.
    fn splash(&self) -> gpui::Div {
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(5.0))
            .font_family(self.fonts.primary.clone())
            .text_size(px(self.font_size))
            .line_height(gpui::relative(1.0))
            .child(gpui::img(assets::LOGO).size(px(200.0)))
            .child(div().italic().text_color(hsla([0xff, 0x87, 0xff])).child("A 90dies mud client written today"))
            .child(div().text_color(hsla([0x88, 0x88, 0x88])).child("/help for how to use clay"))
    }

    fn panel(&self) -> gpui::Div {
        div()
            .p_6()
            .gap_3()
            .flex()
            .flex_col()
            .rounded_lg()
            .bg(hsla([self.theme.status_bar_bg.r, self.theme.status_bar_bg.g, self.theme.status_bar_bg.b]))
            .text_color(hsla(self.palette.fg))
    }

    fn login_screen(&self, multiuser: bool, error: Option<String>, cx: &mut Context<Self>) -> impl IntoElement {
        let mut form = self.panel().w(px(360.0)).child(div().text_size(px(18.0)).child(format!("Log in to {}", self.session.target.key())));
        if multiuser {
            form = form.child(Input::new(&self.login.username));
        }
        form = form.child(Input::new(&self.login.password));
        if let Some(e) = error {
            form = form.child(div().text_size(px(13.0)).text_color(hsla([0xf0, 0xa0, 0x7a])).child(e));
        }
        form = form.child(
            Button::new("login").primary().label("Log in").on_click(cx.listener(|this, _, _window, cx| this.submit_login(cx))),
        );
        div().size_full().flex().items_center().justify_center().child(form)
    }

    /// A follower's, in place of the login or certificate screen (the main window's to answer).
    fn leader_screen(&self) -> impl IntoElement {
        div().size_full().flex().items_center().justify_center().child(self.panel().child("Waiting for the main window to connect."))
    }

    fn mismatch_screen(&self, host: &str, old: &str, new: &str, cx: &mut Context<Self>) -> impl IntoElement {
        let mono = |s: &str| div().font_family(self.fonts.primary.clone()).text_size(px(12.0)).child(s.to_string());
        div().size_full().flex().items_center().justify_center().child(
            self.panel()
                .w(px(560.0))
                .child(div().text_size(px(18.0)).child(format!("The certificate of {host} has changed")))
                .child(div().text_size(px(14.0)).child(
                    "This happens when the server was reinstalled or its certificate regenerated, or when someone is intercepting the connection. Connect only if you know why it changed.",
                ))
                .child(div().text_size(px(13.0)).child("Before:"))
                .child(mono(old))
                .child(div().text_size(px(13.0)).child("Now:"))
                .child(mono(new))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(Button::new("trust").label("Trust the new certificate").on_click(cx.listener(|this, _, _w, cx| {
                            let fx = this.session.trust_cert();
                            this.after(fx, cx);
                        })))
                        .child(Button::new("quit").primary().label("Don't connect").on_click(|_, _w, cx| cx.quit())),
                ),
        )
    }
}

impl Focusable for ClayView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ClayView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(at) = STARTED.get() {
            static LOGGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if !LOGGED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                crate::debug_log(true, &format!("native-gui: first frame {} ms after start", at.elapsed().as_millis()));
            }
        }
        self.run_later(window, cx);
        self.report_view(window);
        let need_login = matches!(self.session.core.status(), Status::NeedLogin { .. });
        if need_login && !self.login_focused {
            // The cursor goes where typing starts: the name in multiuser mode, else the password.
            let multiuser = matches!(self.session.core.status(), Status::NeedLogin { multiuser: true, .. });
            let field = if multiuser && self.login.username.read(cx).value().is_empty() { &self.login.username } else { &self.login.password };
            field.update(cx, |f, cx| f.focus(window, cx));
            self.login_focused = true;
        } else if !need_login && self.login_focused {
            self.login_focused = false;
            window.focus(&self.focus, cx);
        }
        let body = match self.session.core.status().clone() {
            // The main window logs in and answers a changed certificate for every window.
            Status::NeedLogin { .. } | Status::CertMismatch(_) if self.is_follower() => self.leader_screen().into_any_element(),
            Status::NeedLogin { multiuser, error } => self.login_screen(multiuser, error, cx).into_any_element(),
            Status::CertMismatch(m) => self.mismatch_screen(&m.host, &m.old_fingerprint, &m.new_fingerprint, cx).into_any_element(),
            _ if self.session.core.current_world().is_some_and(|w| w.showing_splash) => self.splash().into_any_element(),
            _ => {
                let snapshot = OutputSnapshot {
                    lines: self.visible_lines(),
                    background: self.palette.bg,
                    font_size: self.font_size,
                    fonts: self.fonts.clone(),
                    wrapspace: self.wrapspace,
                    wrap_columns: self.wrap_columns,
                    letter_spacing: self.letter_spacing,
                    word_spacing: self.word_spacing,
                    style_epoch: self.style_epoch,
                    scroll: self.scroll.clone(),
                    selection: self.selection_ranges(),
                    selection_bg: [self.theme.selection_bg.r, self.theme.selection_bg.g, self.theme.selection_bg.b],
                    hover_link: self.select.hover_link.clone(),
                    scrollbar: self.mode.grep.is_some(),
                };
                let link = self.select.hover_link.is_some();
                div()
                    .id("output")
                    .size_full()
                    .map(|d| if link { d.cursor_pointer() } else { d.cursor_text() })
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &MouseDownEvent, _w, cx| this.output_mouse_down(ev, cx)))
                    .on_mouse_down(MouseButton::Right, cx.listener(|this, ev: &MouseDownEvent, _w, cx| this.output_mouse_down(ev, cx)))
                    .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _w, cx| this.output_mouse_move(ev, cx)))
                    .on_mouse_up(MouseButton::Left, cx.listener(|this, ev: &MouseUpEvent, _w, cx| this.output_mouse_up(ev, cx)))
                    .child(output_view(snapshot))
                    .into_any_element()
            }
        };
        // A grep window shows its matches and nothing else (app.js GREP_MODE).
        let grep = self.mode.grep.is_some();
        let icon_bar = if grep { None } else { self.icon_bar(cx) };
        let tabs_top = self.tabs_mode() == "top";
        let mut tabs = if grep { None } else { self.tabs_ribbon(cx) };
        let stats = if grep { None } else { self.stats_panel(cx) };
        let status_bar = (!grep).then(|| self.status_bar(window, cx));
        let show_input = !grep && !matches!(self.session.core.status(), Status::NeedLogin { .. } | Status::CertMismatch(_));
        let input_bar = show_input.then(|| self.input_bar(window, cx));
        let context_menu = self.context_menu(cx);
        let find_popups = self.find_popups(window, cx);
        let input_menu = self.input_menu(cx);
        let debug_dialog = self.debug_dialog(window, cx);
        div()
            .track_focus(&self.focus)
            .key_context(MAIN_CONTEXT)
            .on_action(cx.listener(|this, _: &MainTab, window, cx| this.main_key("tab", window, cx)))
            .on_action(cx.listener(|this, _: &MainTabPrev, window, cx| this.main_key("shift-tab", window, cx)))
            .on_action(cx.listener(|this, _: &MainCopyCtrl, window, cx| this.main_key("ctrl-c", window, cx)))
            .on_action(cx.listener(|this, _: &MainCopyCmd, window, cx| this.main_key("cmd-c", window, cx)))
            .size_full()
            .flex()
            .flex_col()
            .bg(hsla(self.palette.bg))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                // A text field (login, Find, Search) has the keys while it's focused.
                if !this.focus.is_focused(window) {
                    return;
                }
                if this.key_down(&ev.keystroke, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, _window, cx| {
                // Positive is towards the user (down the page in GPUI: content moves up).
                let dy = -f32::from(ev.delta.pixel_delta(px(this.font_size)).y);
                let at_bottom = this.scroll.borrow().at_bottom();
                if dy > 0.0 && at_bottom && this.pending_total() > 0 {
                    // Wheel past the bottom feeds held output in a row at a time.
                    this.more.wheel_px += dy;
                    let rows = (this.more.wheel_px / this.font_size).floor() as usize;
                    if rows > 0 {
                        this.more.wheel_px -= rows as f32 * this.font_size;
                        this.release_lines(rows, cx);
                    }
                } else {
                    this.more.wheel_px = 0.0;
                    let rows = (dy / this.font_size).round() as isize;
                    if rows != 0 {
                        this.user_scroll(rows);
                    }
                }
                cx.notify();
            }))
            // Dragging the font slider, and the end of a press.
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _window, cx| {
                if this.chrome.slider_drag {
                    if ev.pressed_button == Some(MouseButton::Left) {
                        this.slide_to(ev.position.x, cx);
                    } else {
                        this.chrome.slider_drag = false;
                    }
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _window, _cx| {
                    this.chrome.slider_drag = false;
                    this.chrome.closed_by_press = None;
                }),
            )
            .children(icon_bar)
            .when(tabs_top, |d| d.children(tabs.take()))
            .child(div().flex_1().min_h_0().child(body))
            .children(stats)
            .children(tabs)
            .children(status_bar)
            .children(input_bar)
            .children(find_popups)
            .children(context_menu)
            .children(input_menu)
            .children(debug_dialog)
            .children(self.surf.stack.render())
    }
}

gpui::actions!(clay_main, [MainTab, MainTabPrev, MainCopyCtrl, MainCopyCmd]);

const MAIN_CONTEXT: &str = "ClayMain";

/// gpui-component's root binds Tab, Shift+Tab and Copy for itself (focus order, its own
/// text selection), and GPUI runs bindings before key listeners. In the main view they
/// are Clay's keys: bound here in a deeper context, they go to the root only while a text
/// field or a dialog has the focus.
pub(super) fn bind_main_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", MainTab, Some(MAIN_CONTEXT)),
        KeyBinding::new("shift-tab", MainTabPrev, Some(MAIN_CONTEXT)),
        KeyBinding::new("ctrl-c", MainCopyCtrl, Some(MAIN_CONTEXT)),
        KeyBinding::new("cmd-c", MainCopyCmd, Some(MAIN_CONTEXT)),
    ]);
    dialogs::emoji::bind_keys(cx);
    kit::bind_list_keys(cx);
    windows::keybind_editor::bind_keys(cx);
}

impl ClayView {
    fn main_key(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.is_focused(window) {
            cx.propagate();
            return;
        }
        if let Ok(k) = Keystroke::parse(key) {
            self.key_down(&k, window, cx);
        }
    }
}

/// Open the main window and run the session in it.
pub fn run(session: Session, events: mpsc::UnboundedReceiver<NetInput>) {
    let _ = STARTED.set(Instant::now());
    gpui_platform::application().with_assets(assets::Assets).run(move |cx: &mut App| {
        gpui_component::init(cx);
        bind_main_keys(cx);
        // The web client's output is `-webkit-font-smoothing: antialiased`, which WebKit
        // (Linux, macOS) honours with grayscale text. WebView2 is Chromium, which ignores
        // it on Windows and draws ClearType - so there, keep the system's rendering.
        #[cfg(not(target_os = "windows"))]
        cx.set_text_rendering_mode(gpui::TextRenderingMode::Grayscale);
        let fonts = fonts::load(cx);
        let how = ViewWindow { title: "Clay".into(), size: (800.0, 600.0), is_main: true, leader: None };
        let Some((main, main_window)) = open_view_window(cx, fonts, session, Some(events), viewmode::ViewMode::default(), how) else {
            panic!("open the main window");
        };
        // The other windows are this one's: closing it closes Clay.
        let main_id = main_window.window_id();
        cx.on_window_closed(move |cx, id| {
            if id == main_id {
                cx.quit();
            }
        })
        .detach();
        // Redraw when a Discord emoji arrives or an animated one moves to its next frame.
        let (repaint_tx, mut repaint) = mpsc::unbounded();
        emoji::set_repaint(repaint_tx);
        let repaint_view = main.clone();
        cx.spawn(async move |cx| {
            while repaint.next().await.is_some() {
                if repaint_view.update(cx, |v, cx| v.repaint_all(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        cx.activate(true);
    });
}

/// How a window of output opens.
pub(super) struct ViewWindow {
    pub title: SharedString,
    pub size: (f32, f32),
    /// The main window (it keeps the disk cache).
    pub is_main: bool,
    /// A `/window` or `--grep` window: the window whose connection it shares.
    pub leader: Option<gpui::WeakEntity<ClayView>>,
}

/// A window of output: the main window, with its connection's reports on `events`, or a
/// `/window` / `--grep` window following it (see `follow`). Its view and window, or
/// `None` if no window could open.
pub(super) fn open_view_window(
    cx: &mut App,
    fonts: Fonts,
    session: Session,
    events: Option<mpsc::UnboundedReceiver<NetInput>>,
    mode: viewmode::ViewMode,
    how: ViewWindow,
) -> Option<(gpui::WeakEntity<ClayView>, gpui::AnyWindowHandle)> {
    let ViewWindow { title, size: (w, h), is_main, leader } = how;
    // A follower's connection is already up (its leader's): only a window with its own starts one.
    let starts = events.is_some();
    let bounds = Bounds::centered(None, size(px(w), px(h)), cx);
    let mut view = None;
    let window = cx
        .open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions { title: Some(title), ..Default::default() }),
                ..Default::default()
            },
            |window, cx| {
                let v = cx.new(|cx| {
                    let mut v = ClayView::new(fonts, session, window, cx);
                    // Only the main window keeps the disk cache.
                    if is_main {
                        v.cache = Some(CacheSaver::default());
                    }
                    v.mode = mode;
                    v.link.leader = leader;
                    // The window with the connection plays the sounds.
                    if starts {
                        v.media = Some(crate::native_gui::core::media::Media::new(emoji::runtime()));
                    }
                    v
                });
                window.focus(&v.focus_handle(cx), cx);
                view = Some(v.clone());
                // gpui-component's root: its dialogs, notifications and Tab order.
                cx.new(|cx| gpui_component::Root::new(v, window, cx))
            },
        )
        .ok()?;
    let main = view?;
    let weak = main.downgrade();
    // The connection's reports (a follower's come from its leader).
    if let Some(mut events) = events {
        let event_view = weak.clone();
        cx.spawn(async move |cx| {
            while let Some(ev) = events.next().await {
                if event_view.update(cx, |v, cx| v.handle(ev, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }
    // The clock: keepalives, watchdogs, reconnects, chord timeouts.
    let tick_view = weak.clone();
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(Duration::from_millis(250)).await;
            let ticked = window.update(cx, |_, window, cx| tick_view.update(cx, |v, cx| v.tick(window, cx)));
            if !matches!(ticked, Ok(Ok(()))) {
                break;
            }
        }
    })
    .detach();
    if starts {
        main.update(cx, |v, cx| {
            let fx = v.session.start();
            v.after(fx, cx);
        });
    }
    Some((weak, window.into()))
}
