//! The Settings window (app.js `openSettingsPopup`): General, Web and Font tabs and one
//! Save, plus the sidebar's links to the Theme, Keybind and Action editors.
//! - It edits a copy of the settings and sends it back with one `UpdateGlobalSettings`
//!   (the password is never in that copy: blank means unchanged, as app.js sends it).
//! - The Font tab previews on the main window as it goes; Cancel puts the saved font
//!   back.
//! - The Web tab's checks are the console's (`validate_web_settings`), so the three
//!   interfaces block the same saves with the same words.

use gpui::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, KeyDownEvent, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, Subscription, WeakEntity, Window, div, prelude::*, px, relative,
};
use gpui_component::input::{InputEvent, InputState};
use gpui_component::slider::{Slider, SliderEvent, SliderState};

use super::MainEvent;
use crate::native_gui::ui::ClayView;
use crate::native_gui::ui::assets::icon;
use crate::native_gui::ui::dialogs::{self, DialogEvent, Host};
use crate::native_gui::ui::kit::{Btn, Kit, Pick};
use crate::websocket::{GlobalSettingsMsg, WsMessage};

/// A switch of the General tab: its label, its state, and how to flip it.
type Toggle = (&'static str, bool, fn(&mut GlobalSettingsMsg));

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    General,
    Web,
    Font,
}

impl Tab {
    /// `/setup`, `/web`, `/font` and the menu's names for them.
    pub fn from_name(name: &str) -> Tab {
        match name {
            "web" => Tab::Web,
            "font" => Tab::Font,
            _ => Tab::General,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Tab::General => "General",
            Tab::Web => "Web",
            Tab::Font => "Font",
        }
    }
}

/// app.js `FONT_FAMILIES`: (value, label), read out of the web client.
pub fn font_families() -> Vec<(String, String)> {
    let js = crate::http::WEB_APP_JS;
    let Some(start) = js.find("const FONT_FAMILIES = [") else { return Vec::new() };
    let body = &js[start..];
    let body = &body[..body.find("];").unwrap_or(body.len())];
    body.lines()
        .filter_map(|l| {
            let l = l.trim();
            let l = l.strip_prefix("['")?;
            let (value, rest) = l.split_once("', '")?;
            let label = rest.split_once("']")?.0;
            Some((value.to_string(), label.to_string()))
        })
        .collect()
}

/// A stepper's value: (min, max, step) and how it's shown.
fn step(v: f32, by: f32, (min, max): (f32, f32)) -> f32 {
    ((v + by) * 10.0).round().clamp(min * 10.0, max * 10.0) / 10.0
}

struct Fields {
    /// The Kokoro voice name (src/kokoro.rs).
    voice: Entity<InputState>,
    custom_port: Entity<InputState>,
    web_path: Entity<InputState>,
    remote_lines: Entity<InputState>,
    allow_list: Entity<InputState>,
    password: Entity<InputState>,
    cert_file: Entity<InputState>,
    key_file: Entity<InputState>,
    auth_key: Entity<InputState>,
}

pub struct SettingsView {
    main: WeakEntity<ClayView>,
    focus: FocusHandle,
    tab: Tab,
    /// The settings as edited (the font ones also previewed on the main window).
    edit: GlobalSettingsMsg,
    world_switch: Pick,
    theme: Pick,
    tts: Pick,
    speak: Pick,
    tabs: Pick,
    icon_bar: Pick,
    picture_host: Pick,
    port_mode: Pick,
    custom_cert: Pick,
    transparency: Entity<SliderState>,
    fields: Fields,
    fonts: Vec<(String, String, bool)>,
    advanced: bool,
    error: Option<String>,
    saved: bool,
    stack: dialogs::Stack,
    _subs: Vec<Subscription>,
}

impl SettingsView {
    pub fn new(main: WeakEntity<ClayView>, edit: GlobalSettingsMsg, tab: Tab, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let text = |v: String, ph: &str, window: &mut Window, cx: &mut Context<Self>| {
            let ph = ph.to_string();
            cx.new(|cx| InputState::new(window, cx).default_value(v).placeholder(ph))
        };
        let port_mode = if !edit.http_enabled { "disabled" } else if edit.http_port == 9000 { "9000" } else { "custom" };
        // A custom cert the server has but whose paths it didn't send shows "Configured".
        let configured = |path: &str| if edit.tls_configured && path.is_empty() { "Configured" } else { "" };
        let fields = Fields {
            voice: text(edit.tts_voice.clone(), crate::kokoro::DEFAULT_VOICE, window, cx),
            custom_port: text(edit.http_port.to_string(), "", window, cx),
            web_path: text(edit.web_path.clone(), "clay", window, cx),
            remote_lines: text(edit.remote_initial_lines.to_string(), "", window, cx),
            allow_list: text(edit.ws_allow_list.clone(), "192.168.*, *.example.com", window, cx),
            password: text(String::new(), if edit.ws_password_set { "Set - leave blank to keep" } else { "" }, window, cx),
            cert_file: text(edit.ws_cert_file.clone(), if configured(&edit.ws_cert_file).is_empty() { "/path/to/cert.pem" } else { "Configured" }, window, cx),
            key_file: text(edit.ws_key_file.clone(), if configured(&edit.ws_key_file).is_empty() { "/path/to/key.pem" } else { "Configured" }, window, cx),
            auth_key: text(edit.auth_key.clone(), "", window, cx),
        };
        let mut subs = Vec::new();
        for f in [&fields.custom_port, &fields.remote_lines, &fields.password, &fields.cert_file, &fields.key_file] {
            subs.push(cx.subscribe(f, |this, _, ev: &InputEvent, cx| {
                if let InputEvent::Change = ev {
                    this.error = this.web_problem(cx).0;
                    cx.notify();
                }
            }));
        }
        let transparency = cx.new(|_| SliderState::new().min(30.0).max(100.0).step(1.0).default_value((edit.gui_transparency * 100.0).round().clamp(30.0, 100.0)));
        subs.push(cx.subscribe(&transparency, |this, _, ev: &SliderEvent, cx| {
            if let SliderEvent::Change(v) = ev {
                this.edit.gui_transparency = v.start() / 100.0;
                // Live, on every window, as the WebView GUI's slider does; closing without
                // Save puts it back.
                this.preview(cx);
                cx.notify();
            }
        }));
        if let Some(m) = main.upgrade() {
            subs.push(cx.subscribe(&m, |this, m, ev: &MainEvent, cx| {
                // A new auth key (Regen, or another client's): the read-only box follows.
                if let MainEvent::Settings = ev {
                    if let Some(key) = m.read(cx).session.core.settings.as_ref().map(|s| s.auth_key.clone()) {
                        this.edit.auth_key = key;
                    }
                }
                cx.notify();
            }));
        }
        let installed: std::collections::HashSet<String> = cx.text_system().all_font_names().into_iter().map(|n| n.to_lowercase()).collect();
        let mut fonts: Vec<(String, String, bool)> = font_families()
            .into_iter()
            .map(|(v, l)| {
                let ok = v.is_empty() || v.eq_ignore_ascii_case("monospace") || v == "JetBrains Mono" || installed.contains(&v.to_lowercase());
                (v, l, ok)
            })
            .filter(|(v, _, ok)| *ok || *v == edit.font_name)
            .collect();
        if !edit.font_name.is_empty() && !fonts.iter().any(|(v, _, _)| *v == edit.font_name) {
            let ok = installed.contains(&edit.font_name.to_lowercase());
            fonts.push((edit.font_name.clone(), edit.font_name.clone(), ok));
        }
        // Closed without Save, however it was closed (the OS's close button too): the
        // main window's font goes back.
        subs.push(cx.on_release(|this, cx| {
            if !this.saved {
                let _ = this.main.update(cx, |v, cx| v.preview_settings(None, cx));
            }
        }));
        let advanced = edit.web_font_line_height != 1.2 || edit.web_font_letter_spacing != 0.0 || edit.web_font_word_spacing != 0.0;
        let v = Self {
            world_switch: Pick::new(&[("Unseen First", "Unseen First"), ("Alphabetical", "Alphabetical")], &edit.world_switch_mode, window, cx),
            theme: Pick::new(&crate::theme::BUILTIN_THEMES, &edit.gui_theme, window, cx),
            tts: Pick::new(&crate::tts::TtsMode::OPTIONS, &edit.tts_mode, window, cx),
            speak: Pick::new(&[("all", "All"), ("limit", "Limit")], &edit.tts_speak_mode, window, cx),
            tabs: Pick::new(&[("none", "None"), ("top", "Show Top Ribbon"), ("bottom", "Show Bottom Ribbon")], &edit.tabs, window, cx),
            icon_bar: Pick::new(&[("none", "None"), ("app_tablet", "App/Tablet"), ("all", "All")], &edit.icon_bar, window, cx),
            picture_host: Pick::new(crate::picture::PictureService::OPTIONS, &edit.picture_host, window, cx),
            port_mode: Pick::new(&[("disabled", "Disabled"), ("9000", "9000"), ("custom", "Custom")], port_mode, window, cx),
            custom_cert: Pick::new(&[("no", "No"), ("yes", "Yes")], if edit.tls_configured { "yes" } else { "no" }, window, cx),
            main,
            focus: cx.focus_handle(),
            tab,
            edit,
            transparency,
            fields,
            fonts,
            advanced,
            error: None,
            saved: false,
            stack: Default::default(),
            _subs: subs,
        };
        v.preview(cx);
        v
    }

    /// `/setup`, `/web`, `/font` again while the window is open.
    pub fn show(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.tab = tab;
        cx.notify();
    }

    fn text(&self, f: &Entity<InputState>, cx: &App) -> String {
        f.read(cx).value().to_string()
    }

    /// What the Web tab says, and whether Save is blocked.
    fn web_problem(&self, cx: &App) -> (Option<String>, bool) {
        let f = &self.fields;
        let custom = self.custom_cert.value(cx) == "yes";
        let mut cert = self.text(&f.cert_file, cx);
        let mut key = self.text(&f.key_file, cx);
        // Configured but not sent: present, not missing (app.js updateWebValidationMessage).
        if cert.is_empty() && self.edit.tls_configured && self.edit.ws_cert_file.is_empty() {
            cert = "(configured)".into();
        }
        if key.is_empty() && self.edit.tls_configured && self.edit.ws_key_file.is_empty() {
            key = "(configured)".into();
        }
        let password = self.text(&f.password, cx);
        let password = if password.is_empty() && self.edit.ws_password_set { "(unchanged)".to_string() } else { password };
        let v = crate::popup::definitions::web::validate_web_settings(
            &self.port_mode.value(cx),
            &self.text(&f.custom_port, cx),
            &password,
            custom,
            &cert,
            &key,
            &self.text(&f.allow_list, cx),
            &self.text(&f.remote_lines, cx),
        );
        (v.message.map(|m| m.to_string()), v.blocks_save)
    }

    /// Show the font and Transparency as edited.
    fn preview(&self, cx: &mut Context<Self>) {
        let edit = self.edit.clone();
        let _ = self.main.update(cx, |v, cx| v.preview_settings(Some(edit), cx));
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let multiuser = self.main.upgrade().is_some_and(|m| m.read(cx).session.core.is_multiuser());
        let (problem, blocks) = self.web_problem(cx);
        if !multiuser && blocks {
            self.error = problem;
            self.tab = Tab::Web;
            cx.notify();
            return;
        }
        let mut s = self.edit.clone();
        s.world_switch_mode = self.world_switch.value(cx);
        s.gui_theme = self.theme.value(cx);
        s.tts_mode = self.tts.value(cx);
        s.tts_speak_mode = self.speak.value(cx);
        s.tts_voice = crate::kokoro::normalize_voice(&self.text(&self.fields.voice, cx));
        s.tabs = self.tabs.value(cx);
        s.icon_bar = self.icon_bar.value(cx);
        s.picture_host = self.picture_host.value(cx);
        let rows = super::super::input_view::input_height_of(&s).clamp(1, 15);
        super::super::input_view::set_input_height(&mut s, rows);
        s.color_offset_percent = s.color_offset_percent.min(100);
        s.wrapspace = s.wrapspace.min(20);
        let mut password = String::new();
        if !multiuser {
            let f = &self.fields;
            let port_mode = self.port_mode.value(cx);
            s.http_enabled = port_mode != "disabled";
            s.http_port = if port_mode == "custom" { self.text(&f.custom_port, cx).trim().parse().unwrap_or(9000) } else { 9000 };
            s.web_path = self.text(&f.web_path, cx).trim_matches('/').chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-').collect();
            s.ws_allow_list = self.text(&f.allow_list, cx);
            let custom = self.custom_cert.value(cx) == "yes";
            s.ws_cert_file = if custom { self.text(&f.cert_file, cx) } else { String::new() };
            s.ws_key_file = if custom { self.text(&f.key_file, cx) } else { String::new() };
            s.remote_initial_lines = self.text(&f.remote_lines, cx).trim().parse::<i64>().map_or(100, |n| n.clamp(10, 5000) as u16);
            password = self.text(&f.password, cx);
        }
        let mut msg = crate::native_gui::core::client::settings_update(&s);
        if let WsMessage::UpdateGlobalSettings { ws_password, .. } = &mut msg {
            // Blank means unchanged (clearing it is console-only).
            *ws_password = password;
        }
        self.saved = true;
        let _ = self.main.update(cx, |v, cx| {
            // The preview stays until the server's answer replaces it.
            v.keep_preview_until_settings(cx);
            v.send_message(msg, cx);
        });
        let _ = window;
        cx.emit(DialogEvent::Close);
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        cx.emit(DialogEvent::Close);
    }

    /// Import Settings (General; the menu used to have it): this window closes first, as
    /// Cancel does - an import replaces these settings, so its values would be stale and
    /// a Save after it would put the old ones back - and the main window shows the Import
    /// dialog (a dialog there is opened with that window, hence `later`).
    fn import(&mut self, cx: &mut Context<Self>) {
        let _ = self.main.update(cx, |v, cx| {
            v.later(
                |v, window, cx| {
                    window.activate_window();
                    v.open_surface("import", window, cx);
                },
                cx,
            )
        });
        cx.emit(DialogEvent::Close);
    }

    fn help(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = match self.tab {
            Tab::General => "setup",
            Tab::Web => "web",
            Tab::Font => "font",
        };
        let main = self.main.clone();
        dialogs::open_help(self, main, key, window, cx);
    }

    fn open_dialog(&mut self, which: &str, window: &mut Window, cx: &mut Context<Self>) {
        let main = self.main.clone();
        match which {
            "modify-key" => {
                let d = cx.new(|cx| dialogs::web::ModifyKey::new(main, cx));
                dialogs::open(self, dialogs::Kind::ModifyKey, d, window, cx);
            }
            _ => {
                let d = cx.new(|cx| dialogs::web::RemoteAccess::new(main, cx));
                dialogs::open(self, dialogs::Kind::RemoteAccess, d, window, cx);
            }
        }
    }

    fn stepper(&self, k: &Kit, id: &'static str, value: String, minus: fn(&mut Self), plus: fn(&mut Self), cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = &k.t;
        let btn = |id: (&'static str, usize), label: &'static str, f: fn(&mut Self), cx: &mut Context<Self>| {
            let (hover, fg) = (t.hover, t.fg);
            div()
                .id(id)
                .size(px(22.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.0))
                .bg(t.surface)
                .text_color(t.muted)
                .cursor_pointer()
                .hover(move |s| s.bg(hover).text_color(fg))
                .child(label)
                .on_click(cx.listener(move |this, _, _w, cx| {
                    f(this);
                    this.preview(cx);
                    cx.notify();
                }))
        };
        div()
            .flex()
            .items_center()
            .gap(px(10.0))
            .child(btn((id, 0), "−", minus, cx))
            .child(div().min_w(px(28.0)).h(px(20.0)).px(px(4.0)).flex().items_center().justify_center().rounded(px(3.0)).bg(k.field_bg).text_size(px(12.0)).child(value))
            .child(btn((id, 1), "+", plus, cx))
            .into_any_element()
    }

    fn general(&mut self, k: &Kit, cx: &mut Context<Self>) -> gpui::AnyElement {
        let e = &self.edit;
        let offset = if e.color_offset_percent == 0 { "OFF".to_string() } else { format!("{}%", e.color_offset_percent) };
        let (height, wrap) = (super::super::input_view::input_height_of(e).to_string(), e.wrapspace.to_string());
        let (archive, transparency) = (e.scrollback_enabled, (e.gui_transparency * 100.0).round() as u32);
        let toggles = |pairs: Vec<Toggle>, cx: &mut Context<Self>| {
            div().flex_1().flex().flex_col().children(pairs.into_iter().map(|(label, on, flip)| {
                row(k, label, k.toggle(label, on, cx.listener(move |this, _, _w, cx| {
                    flip(&mut this.edit);
                    cx.notify();
                })).into_any_element())
            }))
        };
        let mut left: Vec<Toggle> = vec![
            ("More mode", e.more_mode_enabled, |s| s.more_mode_enabled = !s.more_mode_enabled),
            ("TLS Proxy", e.tls_proxy_enabled, |s| s.tls_proxy_enabled = !s.tls_proxy_enabled),
            ("New Indicator", e.new_line_indicator, |s| s.new_line_indicator = !s.new_line_indicator),
            ("Keyboard Visible", e.keyboard_always_visible, |s| s.keyboard_always_visible = !s.keyboard_always_visible),
            ("Debug", e.debug_enabled, |s| s.debug_enabled = !s.debug_enabled),
            ("Archive Input/Output", e.scrollback_enabled, |s| s.scrollback_enabled = !s.scrollback_enabled),
        ];
        if archive {
            left.push(("Log Input", e.log_input_enabled, |s| s.log_input_enabled = !s.log_input_enabled));
        }
        let right_toggles: Vec<Toggle> = vec![
            ("ANSI Music", e.ansi_music_enabled, |s| s.ansi_music_enabled = !s.ansi_music_enabled),
            ("ZWJ Sequence", e.zwj_enabled, |s| s.zwj_enabled = !s.zwj_enabled),
        ];
        let left = toggles(left, cx);
        let right = toggles(right_toggles, cx)
            .child(row(k, "TTS", self.tts.view()))
            .child(row(k, "Speak Mode", self.speak.view()))
            .child(row(k, "Kokoro Voice", k.input(&self.fields.voice).font_family(k.fonts.ui.clone()).text_size(px(12.0)).into_any_element()))
            .child(row(k, "Tabs", self.tabs.view()))
            .child(row(k, "Icon Bar", self.icon_bar.view()))
            .child(row(k, "Picture Host", self.picture_host.view()));
        // Two cards: the values, then the switches and the remaining choices.
        div()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(
                k.card()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(row(k, "World switching", self.world_switch.view()))
                    .child(row(k, "Theme", self.theme.view()))
                    .child(row(k, "Color Offset", self.stepper(k, "color-offset", offset, |v| v.edit.color_offset_percent = v.edit.color_offset_percent.saturating_sub(5), |v| v.edit.color_offset_percent = (v.edit.color_offset_percent + 5).min(100), cx)))
                    .child(row(k, "Input height", self.stepper(k, "input-height", height, |v| step_input_height(&mut v.edit, -1), |v| step_input_height(&mut v.edit, 1), cx)))
                    .child(row(k, "Wrap Space", self.stepper(k, "wrapspace", wrap, |v| v.edit.wrapspace = v.edit.wrapspace.saturating_sub(1), |v| v.edit.wrapspace = (v.edit.wrapspace + 1).min(20), cx)))
                    .child(row(
                        k,
                        "Transparency",
                        div().flex().items_center().gap(px(8.0)).child(div().flex_1().child(Slider::new(&self.transparency))).child(div().w(px(36.0)).text_size(px(12.0)).child(format!("{transparency}%"))).into_any_element(),
                    )),
            )
            .child(k.card().child(div().flex().gap(px(8.0)).child(left).child(right)))
            .into_any_element()
    }

    fn web(&mut self, k: &Kit, cx: &mut Context<Self>) -> gpui::AnyElement {
        let f = &self.fields;
        let input = |e: &Entity<InputState>| k.input(e).font_family(k.fonts.ui.clone()).text_size(px(12.0)).into_any_element();
        let custom_port = self.port_mode.value(cx) == "custom";
        let custom_cert = self.custom_cert.value(cx) == "yes";
        // Two cards: the server (where it listens), then access (who may reach it).
        let mut server = k.card().flex().flex_col().gap(px(6.0)).child(row(k, "Port", self.port_mode.view()));
        if custom_port {
            server = server.child(row(k, "Custom Port", input(&f.custom_port)));
        }
        server = server
            .child(row(k, "Web Path", input(&f.web_path)))
            .child(row(k, "Remote Lines", div().w(px(80.0)).child(input(&f.remote_lines)).into_any_element()));
        let mut access = k.card().flex().flex_col().gap(px(6.0))
            .child(row(k, "Allow List", input(&f.allow_list)))
            .child(row(k, "Password", input(&f.password)))
            .child(row(k, "Custom Cert File", self.custom_cert.view()));
        if custom_cert {
            access = access.child(row(k, "Cert File", input(&f.cert_file))).child(row(k, "Key File", input(&f.key_file)));
        }
        if let Some(e) = &self.error {
            access = access.child(div().pl(px(98.0)).text_size(px(12.0)).text_color(k.t.error).child(e.clone()));
        }
        access = access
            .child(row(
                k,
                "Auth Key",
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .child(div().flex_1().min_w_0().child(k.input(&f.auth_key).disabled(true).font_family(k.fonts.primary.clone()).text_size(px(10.5))))
                    .child(k.button("web-modify-key", "Modify Key", Btn::Plain, Some(Box::new(cx.listener(|this, _, w, cx| this.open_dialog("modify-key", w, cx))))).px(px(8.0)).py(px(2.0)))
                    .into_any_element(),
            ))
            .child(row(
                k,
                "Remote Access",
                div().flex().child(k.button("web-remote-access", "Open", Btn::Plain, Some(Box::new(cx.listener(|this, _, w, cx| this.open_dialog("remote", w, cx))))).px(px(8.0)).py(px(2.0))).into_any_element(),
            ));
        div().flex().flex_col().gap(px(10.0)).child(server).child(access).into_any_element()
    }

    fn font(&mut self, k: &Kit, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = k.t.clone();
        let e = &self.edit;
        let items = self.fonts.iter().enumerate().map(|(i, (value, label, ok))| {
            let selected = *value == e.font_name;
            let label = if !value.is_empty() && !ok { format!("{label} (not installed)") } else { label.clone() };
            let hover = t.hover;
            let value = value.clone();
            let family: SharedString = if value.is_empty() { k.fonts.primary.clone() } else { value.clone().into() };
            div()
                .id(("font-family", i))
                .px(px(8.0))
                .py(px(5.0))
                .rounded(px(3.0))
                .text_size(px(12.0))
                .whitespace_nowrap()
                .font_family(family)
                .cursor_pointer()
                .when(selected, |d| d.bg(t.accent).text_color(k.field_bg))
                .when(!selected, |d| d.text_color(t.fg).hover(move |s| s.bg(hover)))
                .child(label)
                .on_click(cx.listener(move |this, _, _w, cx| {
                    this.edit.font_name = value.clone();
                    this.preview(cx);
                    cx.notify();
                }))
        });
        let num = |v: f32| format!("{v:.1}");
        let advanced = self.advanced;
        // Three cards: the family, the sizes, the advanced spacing.
        div()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(
                k.card()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(div().text_size(px(12.0)).text_color(t.fg2).child("Font family"))
                    .child(div().id("font-family-list").max_h(px(200.0)).overflow_y_scroll().bg(k.field_bg).border_1().border_color(t.border).rounded(px(6.0)).p(px(2.0)).children(items)),
            )
            .child(
                k.card()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(row(k, "Weight", self.stepper(k, "font-weight", e.web_font_weight.to_string(), |v| v.edit.web_font_weight = v.edit.web_font_weight.saturating_sub(50).max(1), |v| v.edit.web_font_weight = (v.edit.web_font_weight + 50).min(900), cx)))
                    .child(row(k, "Phone", self.stepper(k, "font-phone", (e.web_font_size_phone.round() as i32).to_string(), |v| v.edit.web_font_size_phone = (v.edit.web_font_size_phone.round() - 1.0).max(7.0), |v| v.edit.web_font_size_phone = (v.edit.web_font_size_phone.round() + 1.0).min(20.0), cx)))
                    .child(row(k, "Tablet", self.stepper(k, "font-tablet", (e.web_font_size_tablet.round() as i32).to_string(), |v| v.edit.web_font_size_tablet = (v.edit.web_font_size_tablet.round() - 1.0).max(7.0), |v| v.edit.web_font_size_tablet = (v.edit.web_font_size_tablet.round() + 1.0).min(20.0), cx)))
                    .child(row(k, "Desktop", self.stepper(k, "font-desktop", (e.web_font_size_desktop.round() as i32).to_string(), |v| v.edit.web_font_size_desktop = (v.edit.web_font_size_desktop.round() - 1.0).max(7.0), |v| v.edit.web_font_size_desktop = (v.edit.web_font_size_desktop.round() + 1.0).min(20.0), cx))),
            )
            .child(
                k.card()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(
                        div()
                            .id("font-advanced")
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .text_size(px(11.0))
                            .text_color(t.muted)
                            .cursor_pointer()
                            .child(div().size(px(12.0)).rounded(px(2.0)).border_1().border_color(t.muted).when(advanced, |d| d.bg(t.accent)))
                            .child("Advanced")
                            .on_click(cx.listener(|this, _, _w, cx| {
                                this.advanced = !this.advanced;
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.0))
                            .when(!advanced, |d| d.opacity(0.35))
                            .child(row(k, "Input Ht", self.stepper(k, "font-lineheight", num(e.web_font_line_height), |v| if v.advanced { v.edit.web_font_line_height = step(v.edit.web_font_line_height, -0.1, (0.5, 3.0)) }, |v| if v.advanced { v.edit.web_font_line_height = step(v.edit.web_font_line_height, 0.1, (0.5, 3.0)) }, cx)))
                            .child(row(k, "Letter", self.stepper(k, "font-letter", num(e.web_font_letter_spacing), |v| if v.advanced { v.edit.web_font_letter_spacing = step(v.edit.web_font_letter_spacing, -0.5, (-5.0, 10.0)) }, |v| if v.advanced { v.edit.web_font_letter_spacing = step(v.edit.web_font_letter_spacing, 0.5, (-5.0, 10.0)) }, cx)))
                            .child(row(k, "Word", self.stepper(k, "font-word", num(e.web_font_word_spacing), |v| if v.advanced { v.edit.web_font_word_spacing = step(v.edit.web_font_word_spacing, -0.5, (-5.0, 20.0)) }, |v| if v.advanced { v.edit.web_font_word_spacing = step(v.edit.web_font_word_spacing, 0.5, (-5.0, 20.0)) }, cx))),
                    ),
            )
            .into_any_element()
    }
}

/// `.setting-row`: a right-aligned label and its control.
fn row(k: &Kit, label: &'static str, value: gpui::AnyElement) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .py(px(2.0))
        .child(div().w(px(90.0)).flex_shrink_0().pr(px(8.0)).text_right().text_size(px(12.0)).text_color(k.t.fg2).child(label))
        .child(div().flex_1().min_w_0().child(value))
}

impl Host for SettingsView {
    fn dialogs(&mut self) -> &mut dialogs::Stack {
        &mut self.stack
    }

    fn restore_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
    }
}

impl EventEmitter<DialogEvent> for SettingsView {}
impl Focusable for SettingsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(k) = self.main.upgrade().map(|m| m.read(cx).kit()) else { return div().into_any_element() };
        let t = k.t.clone();
        // The sidebar: the three tabs, then the editors, each an icon over its word.
        let tab_btn = |tab: Tab, ic: &'static str, cx: &mut Context<Self>| {
            let active = self.tab == tab;
            k.side_tile(tab.title(), icon(ic), tab.title(), active).on_click(cx.listener(move |this, _, _w, cx| {
                this.tab = tab;
                cx.notify();
            }))
        };
        let link = |id: &'static str, label: &'static str, ic: &'static str, surface: &'static str, cx: &mut Context<Self>| {
            k.side_tile(id, icon(ic), label, false).on_click(cx.listener(move |this, _, w, cx| {
                let _ = this.main.update(cx, |v, cx| v.open_surface(surface, w, cx));
            }))
        };
        let sidebar = k
            .sidebar()
            .child(tab_btn(Tab::General, "set-general", cx))
            .child(tab_btn(Tab::Web, "set-web", cx))
            .child(tab_btn(Tab::Font, "set-font", cx))
            .child(k.sidebar_divider())
            .child(link("settings-theme", "Theme", "set-theme", "theme-editor", cx))
            .child(link("settings-keybind", "Keybind", "set-keybind", "keybind-editor", cx))
            .child(link("settings-actions", "Actions", "actions", "action-editor", cx));
        let body = match self.tab {
            Tab::General => self.general(&k, cx),
            Tab::Web => self.web(&k, cx),
            Tab::Font => self.font(&k, cx),
        };
        div()
            .id("settings-window")
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .flex()
            .font_family(k.fonts.ui.clone())
            .text_size(px(13.0))
            .line_height(relative(1.4))
            .text_color(t.fg)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _w, cx| {
                if ev.keystroke.key == "escape" {
                    this.cancel(cx);
                    cx.stop_propagation();
                }
            }))
            .child(sidebar)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().px(px(14.0)).pt(px(10.0)).pb(px(4.0)).child(k.header_with(
                        "settings-close",
                        div().text_size(px(11.0)).font_weight(FontWeight::SEMIBOLD).text_color(t.muted).child(self.tab.title().to_uppercase()),
                        cx.listener(|this, _, _, cx| this.cancel(cx)),
                    )))
                    .child(div().id("settings-body").flex_1().min_h_0().overflow_y_scroll().px(px(14.0)).pt(px(8.0)).pb(px(14.0)).child(body))
                    .child(
                        k.buttons()
                            .px(px(14.0))
                            .pt(px(8.0))
                            .pb(px(10.0))
                            .when(self.tab == Tab::General, |d| {
                                d.child(k.button("settings-import", "Import Settings", Btn::Plain, Some(Box::new(cx.listener(|this, _, _, cx| this.import(cx))))))
                            })
                            .child(k.help_button("settings-help", cx.listener(|this, _, w, cx| this.help(w, cx))))
                            .child(div().flex_1())
                            .child(k.button("settings-cancel", "Cancel", Btn::Plain, Some(Box::new(cx.listener(|this, _, _, cx| this.cancel(cx))))))
                            .child(k.button("settings-save", "Save", Btn::Primary, Some(Box::new(cx.listener(|this, _, w, cx| this.save(w, cx)))))),
                    ),
            )
            .children(self.stack.render())
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_font_list_is_app_js_font_families() {
        let fonts = font_families();
        assert_eq!(fonts.first(), Some(&(String::new(), "System Default".to_string())));
        assert!(fonts.iter().any(|(v, l)| v == "ui-monospace" && l == "SF Mono"));
    }

    #[test]
    fn steppers_round_and_clamp() {
        assert_eq!(step(1.2, 0.1, (0.5, 3.0)), 1.3);
        assert_eq!(step(3.0, 0.1, (0.5, 3.0)), 3.0);
        assert_eq!(step(-5.0, -0.5, (-5.0, 10.0)), -5.0);
    }
}

/// The Input height stepper: this GUI's height (`input_view::input_height_of`), 1-15.
fn step_input_height(s: &mut crate::websocket::GlobalSettingsMsg, by: i32) {
    let rows = (i32::from(super::super::input_view::input_height_of(s)) + by).clamp(1, 15) as u16;
    super::super::input_view::set_input_height(s, rows);
}

