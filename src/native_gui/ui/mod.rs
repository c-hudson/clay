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
mod opacity;
mod more;
pub mod output;
mod picture;
mod select;
mod surfaces;
mod textdiv;
mod theme_bridge;
mod titlebar;
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

use crate::native_gui::core::client::{Credentials, Effect, Input as NetInput, Line, Status, hidden_line};
use crate::native_gui::core::keys::{Keys, Resolved};
use crate::native_gui::core::session::Session;
use crate::native_gui::core::decor::{ActionMatcher, DecorOptions, LineFacts, decorate};
use crate::native_gui::core::style::{Palette, StyleOptions, StyledLine};
use crate::native_gui::core::wrap::rows_in_columns;
use crate::theme::ThemeColors;
use chrome::{Chrome, Tokens};
use fonts::Fonts;
use more::MoreMode;
use output::{OutputSnapshot, ScrollHandle, hsla, output_view};

/// A shell world's screen as output rows: each row's id and its styled text.
type ShellRows = Vec<(u64, Arc<StyledLine>)>;

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

/// Lines as the output draws them, decorated once and kept by `Line::id` in
/// `ClayView::styled` (restyled when the line's ▶ comes or goes).
struct Styler<'a> {
    styled: &'a mut HashMap<u64, (bool, Arc<StyledLine>)>,
    palette: &'a Palette,
    style: &'a StyleOptions,
    decor: &'a DecorOptions,
    /// F8: the actions to highlight matches of.
    matcher: Option<&'a ActionMatcher>,
    world: &'a str,
    /// This client's display id (whose ▶ a line carries).
    me: u64,
}

impl Styler<'_> {
    fn line(&mut self, line: &Line) -> Arc<StyledLine> {
        let is_new = self.me != 0 && line.display_id == Some(self.me);
        if let Some((was_new, styled)) = self.styled.get(&line.id) {
            if *was_new == is_new {
                return styled.clone();
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
            action_match: self.matcher.is_some_and(|m| m.matches(&line.text, self.world)),
        };
        let styled = Arc::new(decorate(&facts, self.palette, self.style, self.decor));
        self.styled.insert(line.id, (is_new, styled.clone()));
        styled
    }
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
    /// More-mode per world, by world name (more.rs): the console pages every world.
    more: HashMap<String, MoreMode>,
    /// The text-reveal effect (src/reveal.rs) as the settings and theme have it; None = off.
    reveal: Option<crate::reveal::Reveal>,
    /// When each line was shown live (an arrival, or more-mode letting it through), and
    /// when the view last switched to each world (by name): the later one starts the
    /// line's reveal (`reveal_snapshot`; the console's `World::console_reveal_anchor`).
    reveal_at: HashMap<u64, Instant>,
    reveal_shown: HashMap<String, Instant>,
    keys: Keys,
    /// TF's numeric prefix (`%kbnum`), shown as [N] after the world name.
    kbnum: Option<i64>,
    kbnum_sign: i64,
    /// A shell world (src/shell): a bare Escape was sent to the PTY and we are waiting one
    /// chord window for a possible `Esc-<key>` world switch.
    shell_esc_armed: Option<Instant>,
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
    /// The input height last taken from the server - `input_height_desktop`, this GUI's,
    /// or `input_height` from a server before the split (input_grow/shrink change the rows
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
    /// This GUI has said once that Kokoro is not installed on its machine (`after`).
    kokoro_missing_told: bool,
    /// The voice whose model this GUI asked to have loaded ahead (`warm_kokoro_if_wanted`);
    /// `Some("")` once it found the files missing, so it does not look again until a
    /// local `/tts install` finishes.
    kokoro_warmed: Option<String>,
    /// The current world and whether its media is on, as last seen: turning it off
    /// stops what it started (app.js `GmcpUserToggled`).
    media_gate: Option<(usize, bool)>,
    /// The settings' Transparency, as applied to this window.
    opacity: opacity::Opacity,
    /// The title bar's dark or light, as set for this window (Windows).
    titlebar: titlebar::TitleBar,
    /// Import Picture: this window's pictures going to the server (ui/picture.rs).
    pictures: crate::picture::ClientUploads,
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
        // The system's light/dark changed: GPUI has just put the title bar back to the
        // system's choice, so draw again to put the theme's back (titlebar.rs).
        cx.observe_window_appearance(window, |_, _window, cx| cx.notify()).detach();
        let mut session = session;
        let _ = session.core.set_visible(window.is_visible(), Instant::now());
        let theme = ThemeColors::dark_default();
        let tokens = Tokens::new(&theme, theme.is_light());
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
            more: HashMap::new(),
            reveal: None,
            reveal_at: HashMap::new(),
            reveal_shown: HashMap::new(),
            keys: Keys::default(),
            kbnum: None,
            shell_esc_armed: None,
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
            kokoro_missing_told: false,
            kokoro_warmed: None,
            media_gate: None,
            opacity: Default::default(),
            titlebar: Default::default(),
            pictures: Default::default(),
        };
        view.apply_settings();
        theme_bridge::apply(&view.theme, &view.fonts, cx);
        view.bridged_theme = view.theme_json.clone();
        // A relaunch's input area (/reload, /update), as the console keeps its own.
        view.restore_input_after_relaunch();
        view
    }

    /// What the core's effects mean here. The session does its part first - a `Send`
    /// goes out on the connection (or to a follower's leader) - and the window shows the
    /// rest: a redraw, and the GUI's own messages. An effect the session already applied
    /// passes through unchanged, so a caller may hand this either the core's effects or
    /// the session's leftovers. Applying here is what makes a core call safe to use on
    /// its own: a shell world's keys (`shell_key`/`shell_text`/`shell_paste`) came here
    /// unapplied and were silently never sent.
    fn after(&mut self, effects: Vec<Effect>, cx: &mut Context<Self>) {
        let effects = self.session.apply(effects);
        let mut changed = false;
        let mut local = Vec::new();
        for e in effects {
            match e {
                Effect::Changed => changed = true,
                Effect::Event(msg) => match *msg {
                    crate::websocket::WsMessage::PendingReleased { world_index, count } => {
                        // Released by any interface: that world counts afresh, and shows as
                        // many of its own.
                        let mut released = Vec::new();
                        if let Some(name) = self.session.core.worlds.get(world_index).map(|w| w.name.clone()) {
                            let m = self.more.entry(name).or_default();
                            m.caught_up();
                            m.release(count);
                            released = m.take_released();
                        }
                        self.reveal_stamp(released);
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
                        let settings = self.session.core.settings.as_ref();
                        let mode = crate::tts::TtsMode::from_name(settings.map_or("", |s| s.tts_mode.as_str()));
                        let voice = settings.map_or(crate::kokoro::DEFAULT_VOICE, |s| s.tts_voice.as_str()).to_string();
                        let fell_back = self.media.as_mut().is_some_and(|m| m.speak(&text, mode, &voice));
                        // Attached to another Clay, the voice is this machine's: say once
                        // when Kokoro was chosen but its files aren't here. (Running Clay
                        // itself, the server's own notice covers it - same files.)
                        if fell_back && !self.kokoro_missing_told && !crate::native_gui::runs_clay() {
                            self.kokoro_missing_told = true;
                            self.client_line(Self::KOKORO_MISSING_HERE, cx);
                        }
                    }
                    // The web client passes these to Android's notifications only.
                    crate::websocket::WsMessage::Notification { .. } => {}
                    // What a TF command this window ran does to its input line (/input, /grab,
                    // /dokey, kb*), in order.
                    crate::websocket::WsMessage::InputOps { ops } => {
                        for op in &ops {
                            if !self.editor.apply_input_op(op) {
                                match op {
                                    crate::websocket::InputOp::Action { id } => self.later(
                                        {
                                            let id = id.clone();
                                            move |v, window, cx| v.run_action(&id, window, cx)
                                        },
                                        cx,
                                    ),
                                    crate::websocket::InputOp::Submit => self.later(|v, window, cx| v.send_line(window, cx), cx),
                                    _ => {}
                                }
                            }
                        }
                        changed = true;
                    }
                    // Import Picture: the server's progress and answers (this window's own
                    // uploads only).
                    m @ (crate::websocket::WsMessage::PictureUploadAck { .. } | crate::websocket::WsMessage::PictureUploaded { .. }) => {
                        self.picture_message(m, cx);
                    }
                    // TF's /xtitle run here: this window's title.
                    crate::websocket::WsMessage::SetTitle { title } => {
                        self.later(move |_, window, _| window.set_window_title(&title), cx);
                    }
                    // TF's /limit family run here: this window's filter (F4).
                    crate::websocket::WsMessage::LimitOp { action, pattern, .. } => {
                        self.later(move |v, window, cx| v.apply_limit(&action, &pattern, window, cx), cx);
                    }
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
                Effect::SelfUpdate { force } => self.self_update(force, cx),
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
            self.warm_kokoro_if_wanted();
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

    /// The settings' Transparency (0.3 to 1), the Settings window's preview first.
    pub(super) fn transparency(&self) -> f32 {
        self.surf.preview.as_ref().or(self.session.core.settings.as_ref()).map_or(1.0, |s| s.gui_transparency)
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
            // The world being left counts afresh if it holds nothing (the console's
            // switch_world); the one arrived at shows its own held state (visible_lines
            // stops at its first held line), so a backlog pages as on the console.
            if let Some(old) = self.shown_world.and_then(|i| self.session.core.worlds.get(i)) {
                if let Some(m) = self.more.get_mut(&old.name) {
                    m.left();
                }
            }
            // A real switch - not the first draw after a start or a reload - types the
            // screen it shows on again (the console's World::console_shown_at).
            if self.shown_world.is_some() {
                if let Some(new) = self.session.core.current_world() {
                    self.reveal_shown.insert(new.name.clone(), Instant::now());
                }
            }
            self.shown_world = current;
            // A switch stops the MUD's media; the server sends the new world's (app.js).
            if let Some(m) = self.media.as_mut() {
                m.stop_all();
            }
            self.scroll.borrow_mut().follow_bottom();
        }
        // More-mode counts rows, as the output will wrap the lines: they're measured
        // before they're laid out, since a held line never is.
        let (threshold, cols) = {
            let s = self.scroll.borrow();
            let cols = if self.wrap_columns > 0 { s.view_cols.min(self.wrap_columns) } else { s.view_cols };
            (s.view_rows.saturating_sub(2), cols)
        };
        let (indent, show_tags) = (self.wrapspace, self.show_tags);
        let enabled = self.more_mode_enabled();
        // Every world's arrivals count for its own more-mode, on screen or not, as the
        // console pages every world; only the shown world's pause moves the view.
        let shown = self.session.core.current;
        let mut arrived_all: Vec<u64> = Vec::new();
        for (i, world) in self.session.core.worlds.iter().enumerate() {
            let mut styler = Styler {
                styled: &mut self.styled,
                palette: &self.palette,
                style: &self.style,
                decor: &self.decor,
                matcher: self.action_highlight.then_some(&self.matcher),
                world: &world.name,
                me: self.session.core.display_id,
            };
            let mut rows = |line: &Line| {
                if (line.gagged && !show_tags) || hidden_line(&line.text) {
                    return 0;
                }
                let styled = styler.line(line);
                rows_in_columns(&styled.text, &styled.breaks, cols, indent)
            };
            let m = self.more.entry(world.name.clone()).or_default();
            let paused_now = m.arrivals(world, enabled, threshold, &mut rows);
            arrived_all.extend(m.take_arrived());
            if paused_now && i == shown {
                self.scroll.borrow_mut().follow_bottom();
            }
        }
        // Shown at once: the text-reveal effect types them on from now.
        self.reveal_stamp(arrived_all);
        // Back at the bottom with nothing held: output flows again.
        if self.scroll.borrow().at_bottom() && self.pending_total() == 0 {
            self.more_mut().paused = false;
        }
    }

    /// Lines just shown live (arrivals, or lines more-mode let through): the text-reveal
    /// effect types them on from now. Stamps older than any reveal can last are dropped
    /// first, now and then.
    pub(super) fn reveal_stamp(&mut self, ids: Vec<u64>) {
        if self.reveal.is_none() || ids.is_empty() {
            return;
        }
        let now = Instant::now();
        if self.reveal_at.len() > 512 {
            let stale = std::time::Duration::from_secs_f32(crate::reveal::MAX_SECS + 1.0);
            self.reveal_at.retain(|_, at| now.saturating_duration_since(*at) <= stale);
        }
        for id in ids {
            self.reveal_at.insert(id, now);
        }
    }

    /// The text-reveal effect for this frame: its look, and when each shown line's reveal
    /// started - its live arrival (`reveal_at`) or the switch to its world (`reveal_shown`),
    /// whichever came last. None when it is off, or on a shell world.
    fn reveal_snapshot(&self, lines: &[(u64, Arc<StyledLine>)], shell: bool) -> Option<output::RevealSnapshot> {
        let reveal = self.reveal.filter(|_| !shell)?;
        let now = Instant::now();
        let fresh = |at: &Instant| now.saturating_duration_since(*at).as_secs_f32() <= crate::reveal::MAX_SECS + 1.0;
        let shown = self.session.core.current_world().and_then(|w| self.reveal_shown.get(&w.name)).filter(|at| fresh(at)).copied();
        let mut anchors = HashMap::new();
        for (id, _) in lines {
            let own = self.reveal_at.get(id).filter(|at| fresh(at)).copied();
            let at = match (own, shown) {
                (Some(a), Some(b)) => Some(a.max(b)),
                (a, b) => a.or(b),
            };
            if let Some(at) = at {
                anchors.insert(*id, at);
            }
        }
        Some(output::RevealSnapshot { reveal, anchors })
    }

    /// The shown world's more-mode state, if it has had any arrivals yet.
    fn more(&self) -> Option<&MoreMode> {
        self.session.core.current_world().and_then(|w| self.more.get(&w.name))
    }

    fn more_mut(&mut self) -> &mut MoreMode {
        let name = self.session.core.current_world().map(|w| w.name.clone()).unwrap_or_default();
        self.more.entry(name).or_default()
    }

    /// Whether the shown world's output is paused.
    pub(super) fn more_paused(&self) -> bool {
        self.more().is_some_and(|m| m.paused)
    }

    /// How many lines this window holds back for the shown world.
    pub(super) fn held_here(&self) -> usize {
        self.more().map_or(0, |m| m.held())
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
        // A live shell world (src/shell) captures the keyboard: only the ten world-switch
        // actions run; everything else goes to the PTY. Clipboard shortcuts above still win.
        if self.session.core.current_is_live_shell() {
            use crossterm::event::KeyCode;
            if code == KeyCode::Esc && mods.is_empty() {
                let fx = self.session.core.shell_key(crate::websocket::ShellKey { key: "Escape".into(), ..Default::default() });
                self.after(fx, cx);
                self.shell_esc_armed = Some(Instant::now());
                return true;
            }
            let esc_armed = self.shell_esc_armed.map(|t| t.elapsed() < std::time::Duration::from_millis(500)).unwrap_or(false);
            self.shell_esc_armed = None;
            match crate::shell::keys::classify(code, mods, self.keys.bindings(), esc_armed) {
                crate::shell::keys::Decision::Switch(action) => {
                    self.run_action(&action, window, cx);
                    return true;
                }
                crate::shell::keys::Decision::Forward(k) => {
                    let fx = self.session.core.shell_key(k);
                    self.after(fx, cx);
                    return true;
                }
                crate::shell::keys::Decision::Text(c) => {
                    if c != '\0' {
                        let fx = self.session.core.shell_text(c.to_string());
                        self.after(fx, cx);
                    }
                    return true;
                }
            }
        }
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
                let input = Some(self.input_snapshot());
                let fx = self.session.send(crate::websocket::WsMessage::RunKeyBinding { key, kbnum, input });
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
        // Import Picture: resend after a lost ack; the connection gone, the uploads too.
        if self.pictures.is_busy() && !self.session.core.is_online() {
            self.pictures_connection_lost(cx);
        }
        self.picture_tick(cx);
        self.report_other_views(cx);
        if let Some(m) = self.media.as_mut() {
            m.poll();
        }
        // A chord left waiting fires its own binding.
        match self.keys.tick(Instant::now()) {
            Some(Resolved::Action { action, .. }) => self.run_action(&action, window, cx),
            Some(Resolved::Server(key)) => {
                let input = Some(self.input_snapshot());
                let fx = self.session.send(crate::websocket::WsMessage::RunKeyBinding { key, kbnum: self.kbnum.take(), input });
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
    /// Attached to another Clay with Kokoro as the engine and its files on this machine:
    /// load the model now, once per voice, so the first line spoken does not wait for
    /// it. (Running Clay itself, the server warms its own, the one that speaks.) The
    /// files are looked for once; a local `/tts install` resets that.
    pub(super) fn warm_kokoro_if_wanted(&mut self) {
        if crate::native_gui::runs_clay() {
            return;
        }
        let Some(s) = self.session.core.settings.as_ref() else { return };
        if crate::tts::TtsMode::from_name(&s.tts_mode) != crate::tts::TtsMode::Kokoro {
            // Switched away: let the model and the audio stream go (nothing without an
            // engine), and warm again should the setting come back.
            self.kokoro_warmed = None;
            if let Some(m) = self.media.as_ref() {
                m.release_kokoro();
            }
            return;
        }
        let voice = crate::kokoro::normalize_voice(&s.tts_voice);
        if self.kokoro_warmed.as_deref().is_some_and(|w| w.is_empty() || w == voice) {
            return;
        }
        // No media backend yet (the window attaches it after construction): try again
        // when there is one, so nothing is marked done here.
        let Some(m) = self.media.as_mut() else { return };
        if !crate::kokoro::install().complete() {
            self.kokoro_warmed = Some(String::new());
            return;
        }
        m.warm_kokoro(&voice);
        self.kokoro_warmed = Some(voice);
    }

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
        // The text-reveal effect: the setting's speed (this GUI's own Super Fast), in the
        // theme's colours.
        let mine = s.text_reveal_profiles.get(crate::font_profile::native_key()).map_or("off", String::as_str);
        let cps = crate::TextReveal::from_name(mine).native_chars_per_second();
        self.reveal = (cps > 0.0).then(|| crate::reveal::Reveal::for_gui(cps, self.theme.is_light()));
        // This GUI's fonts on this OS: its own, or the shared ones (src/font_profile.rs).
        let font = s.font_for(crate::font_profile::native_key());
        if font.size_desktop > 0.0 {
            self.font_size = font.size_desktop;
        }
        // Font and wrapping settings change every line's layout.
        let layout = (s.wrapspace as usize, s.wrap_columns, font.letter_spacing, font.word_spacing);
        if layout != (self.wrapspace, self.wrap_columns, self.letter_spacing, self.word_spacing) {
            (self.wrapspace, self.wrap_columns, self.letter_spacing, self.word_spacing) = layout;
            restyle = true;
        }
        restyle |= self.fonts.set_user(&font.name, font.weight);
        let (spell_on, dict) = (s.spell_check_enabled, s.dictionary_path.clone());
        self.input.clear_on_send = s.clear_input_on_send;
        if s.clear_input_on_send {
            self.input.clear_sent();
        }
        let height = input_view::input_height_of(s);
        if height != self.input_height_seen {
            self.input_height_seen = height;
            self.input.rows = (height as usize).clamp(1, 15);
        }
        self.spell_settings(spell_on, &dict);
        let Some(s) = self.surf.preview.as_ref().or(self.session.core.settings.as_ref()) else { return };
        let tokens = Tokens::new(&self.theme, self.theme.is_light());
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

    /// A shell world's screen as output rows plus its cursor (src/shell). Each row's id
    /// mixes its index with a hash of its text, so a row that changes re-lays-out while the
    /// unchanged rows stay cached. `None` if the current world is not a shell world.
    fn shell_output(&self) -> Option<(ShellRows, Option<output::ShellCursor>)> {
        use std::hash::{Hash, Hasher};
        const SHELL_ID_BASE: u64 = 0xE000_0000_0000_0000;
        let screen = self.session.core.current_world()?.shell.as_ref()?;
        let lines = screen
            .lines
            .iter()
            .enumerate()
            .map(|(row, text)| {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                text.hash(&mut h);
                let id = SHELL_ID_BASE | ((row as u64) << 32) | (h.finish() & 0xffff_ffff);
                (id, Arc::new(crate::native_gui::core::style::style_line(text, &self.palette, &self.style)))
            })
            .collect();
        let c = &screen.cursor;
        let cursor = (c.visible && (c.row as usize) < screen.lines.len())
            .then_some(output::ShellCursor { col: c.col, row: c.row, shape: c.shape, color: self.palette.fg });
        Some((lines, cursor))
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
        let mut out = Vec::with_capacity(world.lines.len());
        let mut seqs = Vec::with_capacity(world.lines.len());
        let filter = self.find.filtering().map(str::to_string);
        let cut = self.find.search_cut();
        let mut styler = Styler {
            styled: &mut self.styled,
            palette: &self.palette,
            style: &self.style,
            decor: &self.decor,
            matcher: self.action_highlight.then_some(&self.matcher),
            world: &world.name,
            me: self.session.core.display_id,
        };
        // Nothing below a line more-mode holds back is drawn (more.rs): released lines
        // come out at the bottom, never in above something already shown.
        let mut holding = false;
        let more = self.more.get(&world.name);
        for line in &world.lines {
            holding |= more.is_some_and(|m| m.is_held(line.id));
            if holding || (line.gagged && !self.show_tags) || hidden_line(&line.text) {
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
            out.push((line.id, styler.line(line)));
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
                    // One more than the whole rows that fit: following the bottom, the view
                    // also paints the part-row above them, where the last line would show.
                    let n = self.scroll.borrow().view_rows.max(1) as u64 + 1;
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

    /// Tell the server how much the output shows (it sizes more-mode, history and a shell
    /// world's PTY by it).
    fn report_view(&mut self) {
        // The rows and font cells the output held when last laid out - measured there, not
        // estimated from the window: a column count from the window's width and a nominal
        // cell made a shell's PTY wider than the output beside a panel or in a wider font,
        // wrapping its rows.
        let (rows, cols) = {
            let scroll = self.scroll.borrow();
            (scroll.view_rows.max(1), scroll.view_cols.max(1))
        };
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
        self.report_view();
        // A `/window` takes its main window's, which a Transparency being previewed is on.
        let alpha = self.link.leader.as_ref().and_then(|l| l.upgrade()).map_or_else(|| self.transparency(), |l| l.read(cx).transparency());
        self.opacity.apply(window, alpha);
        self.titlebar.apply(window, !self.chrome.tokens.light);
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
                // A shell world draws its grid with no wrapping and the real cursor; an
                // ordinary world draws its (wrapped, selectable) lines.
                let shell = self.shell_output();
                let (lines, shell_cursor) = match shell {
                    Some((lines, cursor)) => (lines, cursor),
                    None => (self.visible_lines(), None),
                };
                let is_shell = shell_cursor.is_some() || self.session.core.current_world().is_some_and(|w| w.shell.is_some());
                let reveal = self.reveal_snapshot(&lines, is_shell);
                let snapshot = OutputSnapshot {
                    lines,
                    background: self.palette.bg,
                    font_size: self.font_size,
                    fonts: self.fonts.clone(),
                    wrapspace: if is_shell { 0 } else { self.wrapspace },
                    wrap_columns: if is_shell { 0 } else { self.wrap_columns },
                    letter_spacing: self.letter_spacing,
                    word_spacing: self.word_spacing,
                    style_epoch: self.style_epoch,
                    scroll: self.scroll.clone(),
                    selection: self.selection_ranges(),
                    selection_bg: [self.theme.selection_bg.r, self.theme.selection_bg.g, self.theme.selection_bg.b],
                    hover_link: self.select.hover_link.clone(),
                    bar_thumb: self.chrome.tokens.muted,
                    bar_thumb_hot: self.chrome.tokens.fg2,
                    bar_hot: self.select.bar_hot(),
                    shell_cursor,
                    reveal,
                };
                let link = self.select.hover_link.is_some();
                let bar = self.select.bar_hot();
                div()
                    .id("output")
                    .size_full()
                    .map(|d| if bar { d.cursor_default() } else if link { d.cursor_pointer() } else { d.cursor_text() })
                    // Off the output (the bar is on its edge): the bar is no longer lit.
                    .on_hover(cx.listener(|this, hovered: &bool, _w, cx| {
                        if !*hovered && this.select.bar_hover {
                            this.select.bar_hover = false;
                            cx.notify();
                        }
                    }))
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
        let drop_tint = {
            let mut tint = self.chrome.tokens.accent;
            tint.a = 0.12;
            tint
        };
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
                    let font_size = this.font_size;
                    let more = this.more_mut();
                    more.wheel_px += dy;
                    let rows = (more.wheel_px / font_size).floor() as usize;
                    if rows > 0 {
                        more.wheel_px -= rows as f32 * font_size;
                        this.release_rows(rows, cx);
                    }
                } else {
                    this.more_mut().wheel_px = 0.0;
                    let rows = (dy / this.font_size).round() as isize;
                    if rows != 0 {
                        this.user_scroll(rows);
                    }
                }
                cx.notify();
            }))
            // Dragging the font slider, the input's top edge or the scroll bar's thumb, and
            // the end of a press.
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _window, cx| {
                this.bar_drag(ev, cx);
                if this.chrome.slider_drag {
                    if ev.pressed_button == Some(MouseButton::Left) {
                        this.slide_to(ev.position.x, cx);
                    } else {
                        this.chrome.slider_drag = false;
                    }
                }
                if this.input.resize_drag.is_some() {
                    if ev.pressed_button == Some(MouseButton::Left) {
                        this.input_resize_to(f32::from(ev.position.y), cx);
                    } else {
                        this.input_resize_end(cx);
                    }
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _window, cx| {
                    this.chrome.slider_drag = false;
                    this.chrome.closed_by_press = None;
                    this.input_resize_end(cx);
                    this.bar_release(cx);
                }),
            )
            // Import Picture: pictures dropped onto the window (a tint while they're over it).
            .drag_over::<gpui::ExternalPaths>(move |style, _, _, _| style.bg(drop_tint))
            .on_drop(cx.listener(move |this, paths: &gpui::ExternalPaths, _window, cx| {
                // A --grep window has no input line to put the link in.
                if !grep {
                    this.import_pictures(paths.paths().to_vec(), cx);
                }
            }))
            .child(picture::drop_after_typing_fix())
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
    windows::emoji::bind_keys(cx);
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
        let how = ViewWindow { title: "Clay".into(), size: (800.0, 600.0), is_main: true, leader: None, origin: None };
        let Some((main, main_window)) = open_view_window(cx, fonts, session, Some(events), viewmode::ViewMode::default(), how) else {
            panic!("open the main window");
        };
        // Running Clay itself: what /quit does before the process ends (the archive
        // flushed, the TLS proxies ended, the router's port back), as the WebView GUI does.
        if crate::native_gui::runs_clay() {
            cx.on_app_quit(|_| {
                crate::quit_in_process_clay();
                async {}
            })
            .detach();
        }
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
    /// Where its top left goes (`None`: centred on the screen).
    pub origin: Option<gpui::Point<gpui::Pixels>>,
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
    let ViewWindow { title, size: (w, h), is_main, leader, origin } = how;
    // A follower's connection is already up (its leader's): only a window with its own starts one.
    let starts = events.is_some();
    let bounds = match origin {
        Some(origin) => Bounds::new(origin, size(px(w), px(h))),
        None => Bounds::centered(None, size(px(w), px(h)), cx),
    };
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
                        v.warm_kokoro_if_wanted();
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
    main.update(cx, |v, _| v.link.own_window = Some(window.into()));
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
