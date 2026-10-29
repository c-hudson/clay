//! The Theme editor (src/web/theme-editor.html), as a window of its own instead of a
//! page in the browser that logs in again:
//! - the themes and their 42 colours, grouped;
//! - a colour picker (saturation/value square, hue bar, hex);
//! - a live preview, and the console's separator-bar style.
//!
//! Every change is sent at once (`UpdateThemeColors`, so every window sees it); Save
//! writes theme.dat (`SaveThemeFile`).

use std::cell::Cell;
use std::collections::{BTreeMap, HashSet};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    App, AppContext, Bounds, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, ParentElement, Pixels, Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Task, WeakEntity,
    Window, canvas, div, linear_color_stop, linear_gradient, prelude::*, px, relative,
};
use gpui_component::input::{InputEvent, InputState};
use gpui_component::select::{Select, SelectEvent, SelectState};

use super::MainEvent;
use crate::native_gui::ui::ClayView;
use crate::native_gui::ui::dialogs::{self, DialogEvent, Host};
use crate::native_gui::ui::kit::{Btn, Kit};
use crate::native_gui::ui::output::hsla;
use crate::theme::{ThemeColor, ThemeColors};
use crate::websocket::WsMessage;

const ANSI_NAMES: [&str; 16] = [
    "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white", "bright black", "bright red", "bright green", "bright yellow",
    "bright blue", "bright magenta", "bright cyan", "bright white",
];

/// theme-editor.html `VARIABLE_GROUPS`: (group, [(key, what it colours)]).
const GROUPS: &[(&str, &[(&str, &str)])] = &[
    ("Background", &[("bg", "Main content area"), ("bg_deep", "Deepest bg layer"), ("bg_surface", "Slightly elevated"), ("bg_elevated", "Popups, menus"), ("bg_hover", "Hover state")]),
    ("Foreground", &[("fg", "Primary text"), ("fg_secondary", "Less prominent"), ("fg_muted", "Hints"), ("fg_dim", "Disabled")]),
    ("Semantic", &[("accent", "Primary accent"), ("accent_dim", "Dimmed accent"), ("highlight", "Warning/attention"), ("success", "Green"), ("error", "Red"), ("error_dim", "Dimmed error")]),
    (
        "UI Elements",
        &[
            ("status_bar.bg", "Separator bar"),
            ("menu_bar.bg", "Menu/toolbar bar"),
            ("selection.bg", "Text selection"),
            ("link", "URL color"),
            ("prompt", "Input prompt"),
            ("border_subtle", "Subtle borders"),
            ("border_medium", "Medium borders"),
            ("button.selected_bg", "Selected button bg"),
            ("button.selected_fg", "Selected button fg"),
            ("more_indicator.bg", "More badge"),
            ("activity.bg", "Activity indicator"),
        ],
    ),
];

/// The ANSI palette's keys, `ansi.0`..`ansi.15`.
fn ansi_key(i: usize) -> &'static str {
    const KEYS: [&str; 16] = ["ansi.0", "ansi.1", "ansi.2", "ansi.3", "ansi.4", "ansi.5", "ansi.6", "ansi.7", "ansi.8", "ansi.9", "ansi.10", "ansi.11", "ansi.12", "ansi.13", "ansi.14", "ansi.15"];
    KEYS[i]
}

type Colors = BTreeMap<String, String>;

/// A theme's colours over its built-in ones (a custom theme's over dark's).
fn with_defaults(name: &str, given: &serde_json::Map<String, serde_json::Value>) -> Colors {
    let base = ThemeColors::builtin(name).unwrap_or_else(ThemeColors::dark_default);
    let mut out: Colors = serde_json::from_str(&base.to_json()).unwrap_or_default();
    for (k, v) in given {
        if let Some(hex) = v.as_str().filter(|h| ThemeColor::from_hex(h).is_some()) {
            out.insert(k.clone(), hex.to_lowercase());
        }
    }
    out
}

/// Every built-in theme at its defaults (`theme::BUILTIN_THEMES`).
fn builtin_themes() -> BTreeMap<String, Colors> {
    crate::theme::BUILTIN_THEMES.iter().map(|(name, _)| (name.to_string(), defaults(name))).collect()
}

fn defaults(name: &str) -> Colors {
    with_defaults(name, &serde_json::Map::new())
}

fn rgb_of(hex: &str) -> [u8; 3] {
    ThemeColor::from_hex(hex).map_or([0, 0, 0], |c| [c.r, c.g, c.b])
}

/// HSV (hue in degrees, s and v 0..1) of an RGB colour.
pub fn rgb_to_hsv([r, g, b]: [u8; 3]) -> (f32, f32, f32) {
    let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let d = max - min;
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / d).rem_euclid(6.0))
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    (h, if max == 0.0 { 0.0 } else { d / max }, max)
}

pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [u8; 3] {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match (h.rem_euclid(360.0) / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [((r + m) * 255.0).round() as u8, ((g + m) * 255.0).round() as u8, ((b + m) * 255.0).round() as u8]
}

fn hex_of([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Drag {
    Sv,
    Hue,
}

/// A double-clicked colour cycles through the hues for 2 s, to find it in the preview.
struct Flash {
    key: String,
    original: String,
    start: Instant,
}

pub struct ThemeEditor {
    main: WeakEntity<ClayView>,
    focus: FocusHandle,
    themes: BTreeMap<String, Colors>,
    saved: String,
    current: String,
    selected: Option<String>,
    separator: String,
    saved_separator: String,
    loaded: bool,
    collapsed: HashSet<&'static str>,
    hsv: (f32, f32, f32),
    hex: Entity<InputState>,
    sv: Rc<Cell<Bounds<Pixels>>>,
    hue: Rc<Cell<Bounds<Pixels>>>,
    drag: Option<Drag>,
    status: Option<(String, Instant)>,
    flash: Option<Flash>,
    pick: Option<Entity<SelectState<Vec<SharedString>>>>,
    send_task: Option<Task<()>>,
    stack: dialogs::Stack,
    _subs: Vec<Subscription>,
}

impl ThemeEditor {
    pub fn new(main: WeakEntity<ClayView>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let hex = cx.new(|cx| InputState::new(window, cx));
        let subs = vec![cx.subscribe_in(&hex, window, |this, input, ev: &InputEvent, _w, cx| {
            if let InputEvent::Change = ev {
                let v = input.read(cx).value().trim().to_string();
                let v = if v.starts_with('#') { v } else { format!("#{v}") };
                if v.len() == 7 && ThemeColor::from_hex(&v).is_some() {
                    this.hsv = rgb_to_hsv(rgb_of(&v));
                    this.apply(v.to_lowercase(), cx);
                }
            }
        })];
        let _ = main.update(cx, |v, cx| v.send_message(WsMessage::RequestThemeEditorState, cx));
        Self {
            main,
            focus: cx.focus_handle(),
            themes: builtin_themes(),
            saved: String::new(),
            current: "dark".into(),
            selected: None,
            separator: "tinyfugue".into(),
            saved_separator: "tinyfugue".into(),
            loaded: false,
            collapsed: HashSet::new(),
            hsv: (0.0, 1.0, 1.0),
            hex,
            sv: Rc::new(Cell::new(Bounds::default())),
            hue: Rc::new(Cell::new(Bounds::default())),
            drag: None,
            status: None,
            flash: None,
            pick: None,
            send_task: None,
            stack: Default::default(),
            _subs: subs,
        }
    }

    /// The server's themes (`ThemeEditorState`).
    pub fn state(&mut self, themes_json: &str, active: &str, separator: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Ok(serde_json::Value::Object(all)) = serde_json::from_str::<serde_json::Value>(themes_json) {
            self.themes = all.iter().map(|(name, v)| (name.clone(), with_defaults(name, v.as_object().unwrap_or(&serde_json::Map::new())))).collect();
        }
        if self.themes.is_empty() {
            self.themes = builtin_themes();
        }
        self.separator = if separator == "web" { "web".into() } else { "tinyfugue".into() };
        self.saved_separator = self.separator.clone();
        self.current = if self.themes.contains_key(active) { active.into() } else if self.themes.contains_key("dark") { "dark".into() } else { self.themes.keys().next().cloned().unwrap_or_default() };
        self.selected = None;
        self.saved = self.snapshot();
        self.loaded = true;
        self.rebuild_pick(window, cx);
        cx.notify();
    }

    /// Save's answer (`ThemeFileSaved`).
    pub fn saved(&mut self, success: bool, error: Option<&str>, cx: &mut Context<Self>) {
        if success {
            self.saved = self.snapshot();
            self.saved_separator = self.separator.clone();
            self.say("Saved", cx);
        } else {
            self.say(&format!("Error: {}", error.unwrap_or("unknown")), cx);
        }
    }

    fn snapshot(&self) -> String {
        serde_json::to_string(&self.themes).unwrap_or_default()
    }

    fn modified(&self) -> bool {
        self.loaded && (self.snapshot() != self.saved || self.separator != self.saved_separator)
    }

    fn say(&mut self, text: &str, cx: &mut Context<Self>) {
        self.status = Some((text.to_string(), Instant::now()));
        cx.notify();
        cx.spawn(async |this, cx| {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            let _ = this.update(cx, |_, cx| cx.notify());
        })
        .detach();
    }

    fn send(&self, msg: WsMessage, cx: &mut App) {
        let _ = self.main.update(cx, |v, cx| v.send_message(msg, cx));
    }

    /// The theme select, with the theme names as they are now.
    fn rebuild_pick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let names: Vec<SharedString> = self.themes.keys().map(|n| SharedString::from(n.clone())).collect();
        let at = self.themes.keys().position(|n| *n == self.current);
        let state = cx.new(|cx| SelectState::new(names, at.map(gpui_component::IndexPath::new), window, cx));
        cx.subscribe(&state, |this, state, _: &SelectEvent<Vec<SharedString>>, cx| {
            let name = state.read(cx).selected_value().map(|s| s.to_string());
            if let Some(name) = name.filter(|n| this.themes.contains_key(n)) {
                this.current = name;
                this.selected = None;
                cx.notify();
            }
        })
        .detach();
        self.pick = Some(state);
    }

    fn colors(&self) -> Option<&Colors> {
        self.themes.get(&self.current)
    }

    fn select(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = Some(key.to_string());
        let hex = self.colors().and_then(|c| c.get(key)).cloned().unwrap_or_else(|| "#000000".into());
        self.hsv = rgb_to_hsv(rgb_of(&hex));
        self.hex.update(cx, |h, cx| h.set_value(hex, window, cx));
        cx.notify();
    }

    /// A new colour for the selected variable: shown at once, and sent to the server
    /// (debounced 50 ms, as the page does) so every window sees it.
    fn apply(&mut self, hex: String, cx: &mut Context<Self>) {
        let Some(key) = self.selected.clone() else { return };
        let Some(c) = self.themes.get_mut(&self.current) else { return };
        if c.get(&key) == Some(&hex) {
            return;
        }
        c.insert(key, hex);
        cx.notify();
        let (theme_name, colors_json) = (self.current.clone(), serde_json::to_string(c).unwrap_or_default());
        self.send_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(50)).await;
            let _ = this.update(cx, |this, cx| this.send(WsMessage::UpdateThemeColors { theme_name, colors_json }, cx));
        }));
    }

    fn take_hsv(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let hex = hex_of(hsv_to_rgb(self.hsv.0, self.hsv.1, self.hsv.2));
        self.hex.update(cx, |h, cx| h.set_value(hex.clone(), window, cx));
        self.apply(hex, cx);
    }

    fn pointer(&mut self, at: gpui::Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let frac = |b: Bounds<Pixels>| {
            let x = (f32::from(at.x - b.origin.x) / f32::from(b.size.width).max(1.0)).clamp(0.0, 1.0);
            let y = (f32::from(at.y - b.origin.y) / f32::from(b.size.height).max(1.0)).clamp(0.0, 1.0);
            (x, y)
        };
        match self.drag {
            Some(Drag::Sv) => {
                let (x, y) = frac(self.sv.get());
                self.hsv.1 = x;
                self.hsv.2 = 1.0 - y;
            }
            Some(Drag::Hue) => self.hsv.0 = frac(self.hue.get()).0 * 360.0,
            None => return,
        }
        self.take_hsv(window, cx);
    }

    fn flash(&mut self, key: &str, cx: &mut Context<Self>) {
        self.end_flash();
        let Some(original) = self.colors().and_then(|c| c.get(key)).cloned() else { return };
        self.flash = Some(Flash { key: key.to_string(), original, start: Instant::now() });
        cx.spawn(async |this, cx| loop {
            cx.background_executor().timer(Duration::from_millis(40)).await;
            let going = this.update(cx, |this, cx| {
                let Some(f) = &this.flash else { return false };
                let t = f.start.elapsed().as_secs_f32();
                if t >= 2.0 {
                    this.end_flash();
                    cx.notify();
                    return false;
                }
                let (key, hex) = (f.key.clone(), hex_of(hsv_to_rgb((t / 2.0 * 360.0 * 3.0) % 360.0, 1.0, 1.0)));
                if let Some(c) = this.themes.get_mut(&this.current) {
                    c.insert(key, hex);
                }
                cx.notify();
                true
            });
            if !matches!(going, Ok(true)) {
                break;
            }
        })
        .detach();
    }

    fn end_flash(&mut self) {
        if let Some(f) = self.flash.take() {
            if let Some(c) = self.themes.get_mut(&self.current) {
                c.insert(f.key, f.original);
            }
        }
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        // The style first: the server applies it, then writes settings.dat with it.
        if self.separator != self.saved_separator {
            self.send(WsMessage::UpdateSeparatorStyle { style: self.separator.clone() }, cx);
        }
        self.send(WsMessage::SaveThemeFile, cx);
    }

    fn ask(&mut self, text: &str, yes: impl Fn(&mut ThemeEditor, &mut Window, &mut Context<ThemeEditor>) + 'static, window: &mut Window, cx: &mut Context<Self>) {
        let me = cx.entity().downgrade();
        let main = self.main.clone();
        dialogs::confirm(
            self,
            main,
            text.to_string(),
            false,
            move |window, cx| {
                let _ = me.update(cx, |this, cx| yes(this, window, cx));
            },
            window,
            cx,
        );
    }

    fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.modified() {
            return;
        }
        self.ask(
            "Revert all changes since last load/save?",
            |this, window, cx| {
                if let Ok(t) = serde_json::from_str(&this.saved) {
                    this.themes = t;
                }
                this.separator = this.saved_separator.clone();
                if !this.themes.contains_key(&this.current) {
                    this.current = this.themes.keys().next().cloned().unwrap_or_default();
                }
                this.selected = None;
                this.rebuild_pick(window, cx);
                cx.notify();
            },
            window,
            cx,
        );
    }

    fn load_defaults(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let go = |this: &mut ThemeEditor, window: &mut Window, cx: &mut Context<ThemeEditor>| {
            this.themes = builtin_themes();
            this.current = "dark".into();
            this.selected = None;
            for (name, c) in &this.themes {
                this.send(WsMessage::UpdateThemeColors { theme_name: name.clone(), colors_json: serde_json::to_string(c).unwrap_or_default() }, cx);
            }
            this.rebuild_pick(window, cx);
            cx.notify();
        };
        if self.modified() {
            self.ask("Discard current changes and load built-in defaults?", go, window, cx);
        } else {
            go(self, window, cx);
        }
    }

    fn delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.themes.len() <= 1 {
            self.say("Cannot delete the last theme.", cx);
            return;
        }
        let name = self.current.clone();
        self.ask(
            &format!("Delete theme \"{name}\"?"),
            move |this, window, cx| {
                this.send(WsMessage::DeleteTheme { name: name.clone() }, cx);
                this.themes.remove(&name);
                this.current = this.themes.keys().next().cloned().unwrap_or_default();
                this.selected = None;
                this.rebuild_pick(window, cx);
                cx.notify();
            },
            window,
            cx,
        );
    }

    fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let me = cx.entity().downgrade();
        let main = self.main.clone();
        let d = cx.new(|cx| AddTheme::new(main, me, window, cx));
        dialogs::open(self, dialogs::Kind::AddTheme, d, window, cx);
    }

    /// The Add dialog's name (lower-case letters, digits, `_` and `-`).
    fn add_named(&mut self, raw: &str, window: &mut Window, cx: &mut Context<Self>) -> Result<(), String> {
        let name: String = raw.trim().to_lowercase().chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-').collect();
        if name.is_empty() {
            return Err(String::new());
        }
        if self.themes.contains_key(&name) {
            return Err(format!("Theme \"{name}\" already exists."));
        }
        self.send(WsMessage::AddTheme { name: name.clone(), copy_from: self.current.clone() }, cx);
        let copy = self.colors().cloned().unwrap_or_default();
        self.themes.insert(name.clone(), copy);
        self.current = name;
        self.selected = None;
        self.rebuild_pick(window, cx);
        cx.notify();
        Ok(())
    }

    fn color_list(&self, k: &Kit, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = &k.t;
        let Some(colors) = self.colors() else { return div().into_any_element() };
        let mut list = div().flex().flex_col();
        let ansi: Vec<(&str, &str)> = (0..16).map(|i| (ansi_key(i), ANSI_NAMES[i])).collect();
        let groups: Vec<(&'static str, Vec<(&str, &str)>)> = GROUPS.iter().map(|(g, vars)| (*g, vars.to_vec())).chain(std::iter::once(("ANSI Palette", ansi))).collect();
        for (group, vars) in groups {
            let open = !self.collapsed.contains(group);
            list = list.child(
                div()
                    .id(group)
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .px(px(10.0))
                    .py(px(6.0))
                    .text_size(px(11.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.muted)
                    .cursor_pointer()
                    .child(if open { "▼" } else { "▶" })
                    .child(group.to_uppercase())
                    .on_click(cx.listener(move |this, _, _w, cx| {
                        if !this.collapsed.remove(group) {
                            this.collapsed.insert(group);
                        }
                        cx.notify();
                    })),
            );
            if !open {
                continue;
            }
            for (key, desc) in vars {
                let hex = colors.get(key).cloned().unwrap_or_else(|| "#000000".into());
                let selected = self.selected.as_deref() == Some(key);
                let hover = t.hover;
                let key_owned = key.to_string();
                list = list.child(
                    div()
                        .id(SharedString::from(format!("var-{key}")))
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .px(px(10.0))
                        .py(px(4.0))
                        .cursor_pointer()
                        .when(selected, |d| d.bg(t.hover))
                        .when(!selected, |d| d.hover(move |s| s.bg(hover)))
                        .child(div().size(px(18.0)).flex_shrink_0().rounded(px(3.0)).border_1().border_color(t.border).bg(hsla(rgb_of(&hex))))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .min_w_0()
                                .child(div().text_size(px(12.0)).text_color(t.fg).child(key.to_string()))
                                .child(div().text_size(px(10.5)).text_color(t.muted).child(format!("{hex}  {desc}"))),
                        )
                        .on_click(cx.listener(move |this, ev: &gpui::ClickEvent, window, cx| {
                            if ev.click_count() >= 2 {
                                this.flash(&key_owned, cx);
                            } else {
                                this.select(&key_owned, window, cx);
                            }
                        })),
                );
            }
        }
        // The console's separator bar: settings.dat, saved with the theme.
        list = list.child(div().px(px(10.0)).py(px(6.0)).text_size(px(11.0)).font_weight(FontWeight::SEMIBOLD).text_color(t.muted).child("SEPARATOR BAR"));
        for (key, label, desc) in [("tinyfugue", "TinyFugue", "Underscore fill"), ("web", "Clay", "Flat status_bar.bg")] {
            let checked = self.separator == key;
            list = list.child(
                div()
                    .id(key)
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(10.0))
                    .py(px(4.0))
                    .cursor_pointer()
                    .child(div().w(px(18.0)).text_color(if checked { t.accent } else { gpui::transparent_black() }).child("✓"))
                    .child(div().flex().flex_col().child(div().text_size(px(12.0)).child(label)).child(div().text_size(px(10.5)).text_color(t.muted).child(desc)))
                    .on_click(cx.listener(move |this, _, _w, cx| {
                        this.separator = key.to_string();
                        cx.notify();
                    })),
            );
        }
        list.into_any_element()
    }

    fn picker(&self, k: &Kit, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = &k.t;
        let Some(key) = self.selected.clone() else {
            return div().p(px(20.0)).text_color(t.muted).child("Select a color variable to edit").into_any_element();
        };
        let (h, s, v) = self.hsv;
        let pure = hsla(hsv_to_rgb(h, 1.0, 1.0));
        let (sv_cell, hue_cell) = (self.sv.clone(), self.hue.clone());
        let record = |cell: Rc<Cell<Bounds<Pixels>>>| canvas(move |b, _, _| cell.set(b), |_, _, _, _| {}).absolute().size_full();
        let hue_stops = [0.0, 60.0, 120.0, 180.0, 240.0, 300.0, 360.0];
        let current = self.colors().and_then(|c| c.get(&key)).cloned().unwrap_or_default();
        div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .p(px(12.0))
            .child(div().text_size(px(12.0)).text_color(t.fg2).child("Editing: ").child(div().text_color(t.fg).font_weight(FontWeight::BOLD).child(key)).flex())
            .child(
                div()
                    .id("sv")
                    .relative()
                    .w(px(200.0))
                    .h(px(140.0))
                    .bg(pure)
                    .cursor_crosshair()
                    .child(div().absolute().size_full().bg(linear_gradient(90.0, linear_color_stop(gpui::white(), 0.0), linear_color_stop(gpui::Hsla { a: 0.0, ..gpui::white() }, 1.0))))
                    .child(div().absolute().size_full().bg(linear_gradient(180.0, linear_color_stop(gpui::Hsla { a: 0.0, ..gpui::black() }, 0.0), linear_color_stop(gpui::black(), 1.0))))
                    .child(div().absolute().left(px(s * 200.0 - 6.0)).top(px((1.0 - v) * 140.0 - 6.0)).size(px(12.0)).rounded_full().border_2().border_color(gpui::white()))
                    .child(record(sv_cell))
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &MouseDownEvent, w, cx| {
                        this.drag = Some(Drag::Sv);
                        this.pointer(ev.position, w, cx);
                    }))
                    .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, w, cx| {
                        if ev.pressed_button == Some(MouseButton::Left) && this.drag == Some(Drag::Sv) {
                            this.pointer(ev.position, w, cx);
                        }
                    }))
                    .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _w, _cx| this.drag = None)),
            )
            .child(
                div()
                    .id("hue")
                    .relative()
                    .w(px(200.0))
                    .h(px(14.0))
                    .flex()
                    .cursor_pointer()
                    .children(hue_stops.windows(2).map(|w| {
                        div().h_full().w(px(200.0 / 6.0)).bg(linear_gradient(90.0, linear_color_stop(hsla(hsv_to_rgb(w[0], 1.0, 1.0)), 0.0), linear_color_stop(hsla(hsv_to_rgb(w[1] - 0.01, 1.0, 1.0)), 1.0)))
                    }))
                    .child(div().absolute().left(px(h / 360.0 * 200.0 - 2.0)).top(px(-2.0)).w(px(4.0)).h(px(18.0)).rounded(px(2.0)).border_1().border_color(gpui::white()))
                    .child(record(hue_cell))
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &MouseDownEvent, w, cx| {
                        this.drag = Some(Drag::Hue);
                        this.pointer(ev.position, w, cx);
                    }))
                    .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, w, cx| {
                        if ev.pressed_button == Some(MouseButton::Left) && this.drag == Some(Drag::Hue) {
                            this.pointer(ev.position, w, cx);
                        }
                    }))
                    .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _w, _cx| this.drag = None)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(div().w(px(110.0)).child(k.input(&self.hex).font_family(k.fonts.primary.clone())))
                    .child(div().size(px(28.0)).rounded(px(4.0)).border_1().border_color(t.border).bg(hsla(rgb_of(&current)))),
            )
            .into_any_element()
    }

    /// theme-editor.html `updatePreview`: what the theme looks like in use.
    fn preview(&self, k: &Kit) -> gpui::AnyElement {
        let Some(c) = self.colors() else { return div().into_any_element() };
        let col = |key: &str| hsla(rgb_of(c.get(key).map(String::as_str).unwrap_or("#000000")));
        let lines: [(&str, &str, bool, bool); 19] = [
            ("Welcome to the Realm of Shadows!", "ansi.11", true, false),
            ("", "", false, false),
            ("The tavern is dimly lit by flickering candles.", "fg", false, false),
            ("A grizzled dwarf sits in the corner, nursing an ale.", "fg", false, false),
            ("[chat] Gandalf says, \"Anyone seen my staff?\"", "ansi.6", false, false),
            ("[chat] Frodo says, \"Check the lost and found!\"", "ansi.6", false, false),
            ("", "", false, false),
            ("Obvious exits: north, south, east", "ansi.2", false, false),
            ("", "", false, false),
            ("A shadowy figure attacks you!", "ansi.1", true, false),
            ("You dodge the blow and counter-attack.", "ansi.3", false, false),
            ("You deal 42 damage to the shadow.", "success", false, false),
            ("The shadow flees into the darkness.", "fg_secondary", false, false),
            ("", "", false, false),
            ("You gain 150 experience points.", "highlight", false, false),
            ("WARNING: Your health is low!", "error", true, false),
            ("", "", false, false),
            ("Type \"help\" for a list of commands.", "fg_muted", false, false),
            ("Visit https://clay-mud.example.com for more info", "link", false, true),
        ];
        let output = div().flex().flex_col().p(px(8.0)).font_family(k.fonts.primary.clone()).text_size(px(12.0)).line_height(relative(1.35)).children(lines.iter().map(|(text, key, bold, underline)| {
            if text.is_empty() {
                return div().h(px(4.0));
            }
            let d = div().text_color(col(key)).child(text.to_string());
            let d = if *bold { d.font_weight(FontWeight::BOLD) } else { d };
            if *underline { d.underline() } else { d }
        }));
        let sel = col("selection.bg");
        let (fg, fg2, dim) = (col("fg"), col("fg_secondary"), col("fg_dim"));
        let row = |cols: [&str; 4], selected: bool| {
            let widths = [20.0, 92.0, 60.0, 110.0];
            div().flex().px(px(4.0)).py(px(2.0)).when(selected, |d| d.bg(sel)).text_color(if selected { fg } else { fg2 }).children(
                cols.iter().zip(widths).map(|(c, w)| div().w(px(w)).flex_shrink_0().overflow_hidden().whitespace_nowrap().child(c.to_string())),
            )
        };
        let btn = |label: &str, bg: gpui::Hsla, fgc: gpui::Hsla, outline: bool| {
            div().px(px(10.0)).py(px(3.0)).rounded(px(3.0)).bg(bg).text_color(fgc).when(outline, |d| d.border_1().border_color(col("accent"))).child(label.to_string())
        };
        let popup = div()
            .w(px(310.0))
            .p(px(10.0))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .rounded(px(6.0))
            .bg(col("bg_elevated"))
            .text_color(fg)
            .text_size(px(11.0))
            .child(div().font_weight(FontWeight::BOLD).child("Actions"))
            .child(div().px(px(6.0)).py(px(3.0)).rounded(px(3.0)).bg(col("bg_deep")).text_color(dim).child("Filter Actions..."))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .bg(col("bg_surface"))
                    .child(div().flex().px(px(4.0)).text_color(dim).children([("", 20.0), ("Name", 92.0), ("World", 60.0), ("Pattern", 110.0)].map(|(c, w)| div().w(px(w)).child(c))))
                    .child(row(["✓", "auto_heal", "TestMUD", "^You are bleeding"], false))
                    .child(row(["✓", "page_alert", "", "*pages you*"], true))
                    .child(row(["", "highlight_says", "", "* says *"], false))
                    .child(row(["✓", "greet_login", "TestMUD", "^\\w+ has connected"], false))
                    .child(row(["", "spam_filter", "", "^\\[advert\\]"], false)),
            )
            .child(
                div()
                    .flex()
                    .justify_between()
                    .child(btn("Delete", col("error"), fg, false))
                    .child(div().flex().gap(px(4.0)).child(btn("Add", col("bg_hover"), fg2, false)).child(btn("Edit", col("bg_hover"), fg2, false)).child(btn("Ok", col("accent_dim"), col("bg_deep"), true))),
            );
        let menu = div()
            .w(px(150.0))
            .p(px(8.0))
            .flex()
            .flex_col()
            .rounded(px(4.0))
            .bg(col("menu_bar.bg"))
            .text_color(fg)
            .text_size(px(11.0))
            .child(div().font_weight(FontWeight::BOLD).pb(px(4.0)).child("Menu"))
            .children(["Help", "Settings", "Web Settings", "Actions", "World Selector", "Connected Worlds"].iter().enumerate().map(|(i, m)| {
                div().px(px(6.0)).py(px(2.0)).when(i == 1, |d| d.bg(sel).text_color(fg)).when(i != 1, |d| d.text_color(fg2)).child(m.to_string())
            }));
        let now = chrono_now();
        div()
            .flex()
            .flex_col()
            .rounded(px(6.0))
            .overflow_hidden()
            .border_1()
            .border_color(col("border_medium"))
            .bg(col("bg"))
            .child(div().relative().child(output).child(div().absolute().right(px(8.0)).top(px(8.0)).child(popup)).child(div().absolute().left(px(0.0)).bottom(px(0.0)).child(menu)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(8.0))
                    .h(px(24.0))
                    .bg(col("status_bar.bg"))
                    .text_color(fg)
                    .text_size(px(12.0))
                    .child(div().text_color(col("fg_muted")).child("☰"))
                    .child(div().flex().gap(px(4.0)).child(div().text_color(col("success")).child("●")).child("MudWorld"))
                    .child(div().flex_1())
                    .child(div().text_color(col("fg_muted")).child("A 14"))
                    .child(now),
            )
            .child(div().h(px(26.0)).px(px(8.0)).flex().items_center().bg(col("bg_deep")).border_t_1().border_color(col("border_subtle")).child(div().w(px(2.0)).h(px(14.0)).bg(fg)))
            .into_any_element()
    }
}

fn chrono_now() -> String {
    crate::native_gui::ui::chrome::clock_text("")
}

impl Host for ThemeEditor {
    fn dialogs(&mut self) -> &mut dialogs::Stack {
        &mut self.stack
    }

    fn restore_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
    }
}

impl EventEmitter<DialogEvent> for ThemeEditor {}
impl Focusable for ThemeEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ThemeEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(k) = self.main.upgrade().map(|m| m.read(cx).kit()) else { return div().into_any_element() };
        let t = k.t.clone();
        let status = self.status.as_ref().filter(|(_, at)| at.elapsed() < Duration::from_secs(2)).map(|(s, _)| s.clone());
        let button = |id: &'static str, label: &'static str, kind: Btn, f: fn(&mut ThemeEditor, &mut Window, &mut Context<ThemeEditor>), cx: &mut Context<Self>| {
            k.button(id, label, kind, Some(Box::new(cx.listener(move |this, _, w, cx| f(this, w, cx))))).px(px(10.0)).py(px(3.0))
        };
        let toolbar = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.0))
            .px(px(12.0))
            .py(px(8.0))
            .border_b_1()
            .border_color(t.border)
            .child(div().font_weight(FontWeight::BOLD).child("Clay Theme Editor"))
            .when(self.modified(), |d| d.child(div().size(px(8.0)).rounded_full().bg(t.highlight)))
            .children(status.map(|s| div().text_size(px(12.0)).text_color(t.success).child(s)))
            .child(div().flex_1())
            .child(div().text_size(px(12.0)).text_color(t.fg2).child("Theme:"))
            .children(self.pick.as_ref().map(|p| div().w(px(130.0)).child(Select::new(p).w_full())))
            .child(button("theme-add", "Add", Btn::Plain, |this, w, cx| this.add(w, cx), cx))
            .child(button("theme-delete", "Delete", Btn::Danger, |this, w, cx| this.delete(w, cx), cx))
            .child(button("theme-save", "Save", Btn::Primary, |this, _w, cx| this.save(cx), cx))
            .child(button("theme-reset", "Reset", Btn::Plain, |this, w, cx| this.reset(w, cx), cx))
            .child(button("theme-defaults", "Load Defaults", Btn::Plain, |this, w, cx| this.load_defaults(w, cx), cx));
        let body = div()
            .flex_1()
            .min_h_0()
            .flex()
            .child(div().id("theme-vars").w(px(260.0)).flex_shrink_0().overflow_y_scroll().border_r_1().border_color(t.border).py(px(4.0)).child(self.color_list(&k, cx)))
            .child(
                div()
                    .id("theme-right")
                    .flex_1()
                    .min_w_0()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .child(self.picker(&k, cx))
                    .child(div().px(px(12.0)).pb(px(12.0)).flex().flex_col().gap(px(6.0)).child(div().text_size(px(11.0)).text_color(t.muted).child("LIVE PREVIEW")).child(self.preview(&k))),
            );
        div()
            .id("theme-editor")
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .font_family(k.fonts.ui.clone())
            .text_size(px(13.0))
            .text_color(t.fg)
            .child(toolbar)
            .child(body)
            .children(self.stack.render())
            .into_any_element()
    }
}

/// Add New Theme: a name.
pub struct AddTheme {
    main: WeakEntity<ClayView>,
    editor: WeakEntity<ThemeEditor>,
    name: Entity<InputState>,
    error: Option<String>,
}

impl AddTheme {
    fn new(main: WeakEntity<ClayView>, editor: WeakEntity<ThemeEditor>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("Theme name (e.g. midnight)"));
        cx.subscribe_in(&name, window, |this, _, ev: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { .. } = ev {
                this.confirm(window, cx);
            }
        })
        .detach();
        Self { main, editor, name, error: None }
    }

    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let raw = self.name.read(cx).value().to_string();
        let r = self.editor.update(cx, |e, cx| e.add_named(&raw, window, cx));
        match r {
            Ok(Ok(())) => cx.emit(DialogEvent::Close),
            Ok(Err(e)) => {
                self.error = Some(e).filter(|e| !e.is_empty());
                cx.notify();
            }
            Err(_) => cx.emit(DialogEvent::Close),
        }
    }
}

impl EventEmitter<DialogEvent> for AddTheme {}
impl Focusable for AddTheme {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.name.read(cx).focus_handle(cx)
    }
}

impl Render for AddTheme {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(k) = self.main.upgrade().map(|m| m.read(cx).kit()) else { return div().into_any_element() };
        k.panel("add-theme")
            .w(px(320.0))
            .on_key_down(cx.listener(|_, ev: &gpui::KeyDownEvent, _w, cx| {
                if ev.keystroke.key == "escape" {
                    cx.emit(DialogEvent::Close);
                    cx.stop_propagation();
                }
            }))
            .child(div().font_weight(FontWeight::BOLD).child("Add New Theme"))
            .child(k.input(&self.name))
            .child(k.error(self.error.as_deref()))
            .child(
                k.buttons()
                    .child(k.button("add-theme-cancel", "Cancel", Btn::Plain, Some(Box::new(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))))))
                    .child(k.button("add-theme-ok", "Add", Btn::Primary, Some(Box::new(cx.listener(|this, _, w, cx| this.confirm(w, cx)))))),
            )
            .into_any_element()
    }
}

impl ThemeEditor {
    /// Redraw on the main view's news (a live edit's colours coming back).
    pub fn follow(&mut self, cx: &mut Context<Self>) {
        if let Some(m) = self.main.upgrade() {
            self._subs.push(cx.subscribe(&m, |_, _, _: &MainEvent, cx| cx.notify()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_round_trips() {
        for rgb in [[0x13, 0x19, 0x26], [0xd4, 0x84, 0x5a], [0, 0, 0], [255, 255, 255], [0x08, 0x91, 0xb2]] {
            let (h, s, v) = rgb_to_hsv(rgb);
            assert_eq!(hsv_to_rgb(h, s, v), rgb, "{rgb:?}");
        }
        assert_eq!(hsv_to_rgb(0.0, 1.0, 1.0), [255, 0, 0]);
        assert_eq!(hsv_to_rgb(120.0, 1.0, 1.0), [0, 255, 0]);
    }

    #[test]
    fn a_theme_fills_in_from_the_built_in_one() {
        let mut given = serde_json::Map::new();
        given.insert("bg".into(), serde_json::Value::String("#ABCDEF".into()));
        given.insert("bogus".into(), serde_json::Value::String("nothex".into()));
        let c = with_defaults("dark", &given);
        assert_eq!(c.get("bg").map(String::as_str), Some("#abcdef"));
        assert!(c.contains_key("ansi.15") && c.contains_key("status_bar.bg"), "every key the built-in theme has");
        assert_eq!(c.len(), 26 + 16);
    }
}
