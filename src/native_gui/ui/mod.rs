//! The GPUI side of the native GUI.

pub mod fonts;
pub mod output;
#[cfg(feature = "native-gui-snapshot")]
pub mod snapshot;

use std::sync::Arc;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{
    App, AppContext, Bounds, Context, FocusHandle, Focusable, IntoElement, KeyDownEvent, ParentElement, Render,
    ScrollWheelEvent, SharedString, Styled, Window, WindowBounds, WindowOptions, div, prelude::*, px, size,
};

use crate::native_gui::core::net::{NetEvent, Sender};
use crate::native_gui::core::style::{Palette, StyleOptions, StyledLine, strip_mud_tag, style_line};
use crate::theme::ThemeColors;
use crate::websocket::WsMessage;
use fonts::Fonts;
use output::{OutputSnapshot, hsla, output_view};

/// `web_font_size_desktop`'s default.
const DEFAULT_FONT_SIZE: f32 = 18.0;
/// `--status-height` in style.css.
const STATUS_HEIGHT: f32 = 26.0;

struct WorldView {
    name: String,
    lines: Vec<Arc<StyledLine>>,
    /// Text received without its newline yet (app.js `partialLines`): never drawn.
    partial: String,
}

pub struct ClayView {
    focus: FocusHandle,
    fonts: Fonts,
    theme: ThemeColors,
    palette: Palette,
    style: StyleOptions,
    font_size: f32,
    wrapspace: usize,
    worlds: Vec<WorldView>,
    current: usize,
    scroll_rows: usize,
    status: SharedString,
    _sender: Sender,
}

impl ClayView {
    fn new(fonts: Fonts, sender: Sender, cx: &mut Context<Self>) -> Self {
        let theme = ThemeColors::dark_default();
        ClayView {
            focus: cx.focus_handle(),
            fonts,
            palette: Palette::from_theme(&theme),
            theme,
            style: StyleOptions::default(),
            font_size: DEFAULT_FONT_SIZE,
            wrapspace: 0,
            worlds: Vec::new(),
            current: 0,
            scroll_rows: 0,
            status: "Connecting…".into(),
            _sender: sender,
        }
    }

    fn style(&self, raw: &str) -> Arc<StyledLine> {
        style_raw(raw, &self.palette, &self.style)
    }

    pub(super) fn handle(&mut self, event: NetEvent) {
        match event {
            NetEvent::Closed(reason) => self.status = format!("Disconnected: {reason}").into(),
            NetEvent::Message(msg) => self.handle_message(*msg),
        }
    }

    fn apply_settings(&mut self, settings: &crate::websocket::GlobalSettingsMsg) {
        if !settings.theme_colors_json.is_empty() {
            self.theme = ThemeColors::from_json(&settings.theme_colors_json, &ThemeColors::dark_default());
            self.palette = Palette::from_theme(&self.theme);
        }
        if settings.web_font_size_desktop > 0.0 {
            self.font_size = settings.web_font_size_desktop;
        }
        self.style.color_offset_percent = settings.color_offset_percent as u32;
    }

    fn handle_message(&mut self, msg: WsMessage) {
        match msg {
            WsMessage::InitialState { worlds, settings, current_world_index, .. } => {
                self.apply_settings(&settings);
                self.worlds = worlds
                    .iter()
                    .map(|w| WorldView {
                        name: w.name.clone(),
                        lines: w.output_lines_ts.iter().filter(|l| !l.gagged).map(|l| self.style(&l.text)).collect(),
                        partial: String::new(),
                    })
                    .collect();
                self.current = current_world_index.min(self.worlds.len().saturating_sub(1));
                self.status = self.world_name().into();
            }
            WsMessage::GlobalSettingsUpdated { settings, .. } => self.apply_settings(&settings),
            WsMessage::ServerData { world_index, data, gagged, .. } => {
                if gagged {
                    return;
                }
                let Some(world) = self.worlds.get_mut(world_index) else { return };
                let mut text = std::mem::take(&mut world.partial);
                text.push_str(&data);
                let complete = text.ends_with('\n');
                let mut parts: Vec<&str> = text.split('\n').collect();
                let tail = parts.pop().unwrap_or("");
                let mut styled = Vec::with_capacity(parts.len());
                for part in parts {
                    let raw = part.strip_suffix('\r').unwrap_or(part);
                    styled.push(style_raw(raw, &self.palette, &self.style));
                }
                let world = &mut self.worlds[world_index];
                world.lines.extend(styled);
                if !complete {
                    world.partial = tail.to_string();
                }
            }
            WsMessage::WorldSwitched { new_index, .. } => {
                if new_index < self.worlds.len() {
                    self.current = new_index;
                    self.scroll_rows = 0;
                    self.status = self.world_name().into();
                }
            }
            _ => {}
        }
    }

    fn world_name(&self) -> String {
        self.worlds.get(self.current).map(|w| w.name.clone()).unwrap_or_default()
    }

    fn page_rows(&self, window: &Window) -> usize {
        let h = f32::from(window.viewport_size().height) - STATUS_HEIGHT;
        ((h / self.font_size).floor() as usize).saturating_sub(2).max(1)
    }
}

/// One server line as the output shows it: the MUD tag stripped (tags are hidden until
/// F2 exists here), then styled.
fn style_raw(raw: &str, palette: &Palette, style: &StyleOptions) -> Arc<StyledLine> {
    Arc::new(style_line(&strip_mud_tag(raw), palette, style))
}

impl Focusable for ClayView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ClayView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let lines = self.worlds.get(self.current).map(|w| w.lines.clone()).unwrap_or_default();
        let snapshot = OutputSnapshot {
            lines,
            background: self.palette.bg,
            font_size: self.font_size,
            fonts: self.fonts.clone(),
            scroll_rows: self.scroll_rows,
            wrapspace: self.wrapspace,
        };
        let status_bg = [self.theme.status_bar_bg.r, self.theme.status_bar_bg.g, self.theme.status_bar_bg.b];
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
            .child(div().flex_1().min_h_0().child(output_view(snapshot)))
            .child(
                div()
                    .h(px(STATUS_HEIGHT))
                    .px_2()
                    .flex()
                    .items_center()
                    .bg(hsla(status_bg))
                    .text_color(hsla(self.palette.fg))
                    .text_size(px(13.0))
                    .child(self.status.clone()),
            )
    }
}

/// Open the main window and pump connection events into it.
pub fn run(sender: Sender, events: mpsc::UnboundedReceiver<NetEvent>) {
    gpui_platform::application().run(move |cx: &mut App| {
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
                    let view = cx.new(|cx| ClayView::new(fonts, sender, cx));
                    window.focus(&view.focus_handle(cx), cx);
                    view
                },
            )
            .expect("open the main window");
        let view = window.update(cx, |_, _, cx| cx.entity().downgrade()).expect("main view");
        let mut events = events;
        cx.spawn(async move |cx| {
            while let Some(ev) = events.next().await {
                if view.update(cx, |v, cx| {
                    v.handle(ev);
                    cx.notify();
                }).is_err() {
                    break;
                }
            }
        })
        .detach();
        cx.activate(true);
    });
}
