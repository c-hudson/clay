//! The GPUI side of the native GUI.

pub mod emoji;
pub mod fonts;
pub mod ink;
pub mod output;
#[cfg(feature = "native-gui-snapshot")]
pub mod snapshot;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{
    App, AppContext, Bounds, Context, Entity, FocusHandle, Focusable, IntoElement, KeyDownEvent, ParentElement, Render,
    ScrollWheelEvent, SharedString, Styled, Window, WindowBounds, WindowOptions, div, prelude::*, px, size,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::input::{Input, InputEvent, InputState};

use crate::native_gui::core::client::{Credentials, Effect, Input as NetInput, Status};
use crate::native_gui::core::session::Session;
use crate::native_gui::core::style::{Palette, StyleOptions, StyledLine, strip_mud_tag, style_line};
use crate::theme::ThemeColors;
use fonts::Fonts;
use output::{OutputSnapshot, hsla, output_view};

/// `web_font_size_desktop`'s default.
const DEFAULT_FONT_SIZE: f32 = 18.0;
/// `--status-height` in style.css.
const STATUS_HEIGHT: f32 = 26.0;

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
    /// Styled lines by `Line::id`, valid while the style inputs are unchanged.
    styled: HashMap<u64, Arc<StyledLine>>,
    scroll_rows: usize,
    login: LoginForm,
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
        let theme = ThemeColors::dark_default();
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
            styled: HashMap::new(),
            scroll_rows: 0,
            login: LoginForm { username, password },
        };
        view.apply_settings();
        view
    }

    /// Effects the session left for us: redraw, and (later) the GUI's own messages.
    fn after(&mut self, effects: Vec<Effect>, cx: &mut Context<Self>) {
        let mut changed = false;
        for e in effects {
            match e {
                Effect::Changed => changed = true,
                Effect::Event(_msg) => {}
                _ => {}
            }
        }
        if changed {
            self.apply_settings();
            cx.notify();
        }
    }

    pub(super) fn handle(&mut self, input: NetInput, cx: &mut Context<Self>) {
        let fx = self.session.handle(input);
        self.after(fx, cx);
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        let fx = self.session.tick();
        self.after(fx, cx);
        if matches!(self.session.core.status(), Status::Reconnecting { .. }) {
            cx.notify(); // the countdown
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
        let Some(s) = self.session.core.settings.as_ref() else { return };
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
        let offset = s.color_offset_percent as u32;
        if offset != self.style.color_offset_percent || s.show_tags != self.show_tags {
            self.style.color_offset_percent = offset;
            self.show_tags = s.show_tags;
            self.style.discord_emoji = !s.show_tags;
            restyle = true;
        }
        if restyle {
            self.styled.clear();
        }
    }

    /// The current world's lines as the output shows them.
    fn visible_lines(&mut self) -> Vec<Arc<StyledLine>> {
        let Some(world) = self.session.core.current_world() else { return Vec::new() };
        let mut out = Vec::with_capacity(world.lines.len());
        for line in &world.lines {
            if line.gagged && !self.show_tags {
                continue;
            }
            let styled = self
                .styled
                .entry(line.id)
                .or_insert_with(|| style_raw(&line.text, &self.palette, &self.style, self.show_tags))
                .clone();
            out.push(styled);
        }
        if self.styled.len() > 2 * out.len() + 1024 {
            let live: std::collections::HashSet<u64> = world.lines.iter().map(|l| l.id).collect();
            self.styled.retain(|id, _| live.contains(id));
        }
        out
    }

    fn status_text(&self) -> String {
        let core = &self.session.core;
        let world = core.current_world().map(|w| w.name.clone()).unwrap_or_default();
        match core.status() {
            Status::Online => world,
            Status::Idle | Status::Connecting { .. } => format!("Connecting to {}…", self.session.target.key()),
            Status::LoggingIn => "Logging in…".into(),
            Status::NeedLogin { .. } => "Log in".into(),
            Status::CertMismatch(_) => "The server's certificate has changed".into(),
            Status::Reconnecting { at, reason, .. } => {
                let secs = at.saturating_duration_since(Instant::now()).as_secs_f32().ceil() as u64;
                format!("{world}  —  reconnecting in {secs} s ({reason})")
            }
            Status::Stopped(why) => why.clone(),
        }
    }

    fn page_rows(&self, window: &Window) -> usize {
        let h = f32::from(window.viewport_size().height) - STATUS_HEIGHT;
        ((h / self.font_size).floor() as usize).saturating_sub(2).max(1)
    }

    /// Tell the server how much the output shows (it sizes more-mode and history by it).
    fn report_view(&mut self, window: &Window) {
        let vp = window.viewport_size();
        let rows = ((f32::from(vp.height) - STATUS_HEIGHT - 8.0) / self.font_size).floor().max(1.0) as usize;
        let cols = ((f32::from(vp.width) - 8.0) / (self.font_size * 0.6)).floor().max(1.0) as usize;
        let fx = self.session.core.set_view(rows, cols);
        let _ = self.session.apply(fx);
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

/// One server line as the output shows it: the MUD tag stripped unless tags are shown
/// (F2), then styled.
fn style_raw(raw: &str, palette: &Palette, style: &StyleOptions, show_tags: bool) -> Arc<StyledLine> {
    if show_tags { Arc::new(style_line(raw, palette, style)) } else { Arc::new(style_line(&strip_mud_tag(raw), palette, style)) }
}

impl Focusable for ClayView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ClayView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.report_view(window);
        let status_bg = [self.theme.status_bar_bg.r, self.theme.status_bar_bg.g, self.theme.status_bar_bg.b];
        let status = self.status_text();
        let body = match self.session.core.status().clone() {
            Status::NeedLogin { multiuser, error } => self.login_screen(multiuser, error, cx).into_any_element(),
            Status::CertMismatch(m) => self.mismatch_screen(&m.host, &m.old_fingerprint, &m.new_fingerprint, cx).into_any_element(),
            _ => {
                let snapshot = OutputSnapshot {
                    lines: self.visible_lines(),
                    background: self.palette.bg,
                    font_size: self.font_size,
                    fonts: self.fonts.clone(),
                    scroll_rows: self.scroll_rows,
                    wrapspace: self.wrapspace,
                };
                output_view(snapshot).into_any_element()
            }
        };
        div()
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(hsla(self.palette.bg))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                let page = this.page_rows(window);
                match ev.keystroke.key.as_str() {
                    "pageup" => this.scroll_rows += page,
                    "pagedown" => this.scroll_rows = this.scroll_rows.saturating_sub(page),
                    "end" => this.scroll_rows = 0,
                    _ => return,
                }
                cx.notify();
            }))
            .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, _window, cx| {
                let dy = f32::from(ev.delta.pixel_delta(px(this.font_size)).y);
                let rows = (dy / this.font_size).round() as isize;
                this.scroll_rows = (this.scroll_rows as isize + rows).max(0) as usize;
                cx.notify();
            }))
            .child(div().flex_1().min_h_0().child(body))
            .child(
                div()
                    .h(px(STATUS_HEIGHT))
                    .px_2()
                    .flex()
                    .items_center()
                    .bg(hsla(status_bg))
                    .text_color(hsla(self.palette.fg))
                    .text_size(px(13.0))
                    .child(SharedString::from(status)),
            )
    }
}

/// Open the main window and run the session in it.
pub fn run(session: Session, events: mpsc::UnboundedReceiver<NetInput>) {
    gpui_platform::application().run(move |cx: &mut App| {
        gpui_component::init(cx);
        // The web client's output is `-webkit-font-smoothing: antialiased`, which WebKit
        // (Linux, macOS) honours with grayscale text. WebView2 is Chromium, which ignores
        // it on Windows and draws ClearType - so there, keep the system's rendering.
        #[cfg(not(target_os = "windows"))]
        cx.set_text_rendering_mode(gpui::TextRenderingMode::Grayscale);
        let fonts = fonts::load(cx);
        let bounds = Bounds::centered(None, size(px(800.0), px(600.0)), cx);
        let window = cx
            .open_window(
                WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() },
                |window, cx| {
                    let view = cx.new(|cx| ClayView::new(fonts, session, window, cx));
                    window.focus(&view.focus_handle(cx), cx);
                    view
                },
            )
            .expect("open the main window");
        let view = window.update(cx, |_, _, cx| cx.entity().downgrade()).expect("main view");
        // Redraw when a Discord emoji arrives or an animated one moves to its next frame.
        let (repaint_tx, mut repaint) = mpsc::unbounded();
        emoji::set_repaint(repaint_tx);
        let repaint_view = view.clone();
        cx.spawn(async move |cx| {
            while repaint.next().await.is_some() {
                if repaint_view.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
        .detach();
        // The connection's reports.
        let event_view = view.clone();
        let mut events = events;
        cx.spawn(async move |cx| {
            while let Some(ev) = events.next().await {
                if event_view.update(cx, |v, cx| v.handle(ev, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        // The clock: keepalives, watchdogs, reconnects.
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                if view.update(cx, |v, cx| v.tick(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        window
            .update(cx, |v, _, cx| {
                let fx = v.session.start();
                v.after(fx, cx);
            })
            .expect("start the session");
        cx.activate(true);
    });
}
