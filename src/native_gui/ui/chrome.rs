//! The window around the output, drawn as the web client draws it (style.css,
//! index.html): the status bar, its two menus and font slider, and the colours all of the
//! chrome shares (`Tokens`). The icon bar, tabs ribbon and stats panel are in `bars`.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

use gpui::{
    AnyElement, Anchor, Bounds, BoxShadow, Context, FontWeight, Hsla, IntoElement, MouseButton, MouseDownEvent, ParentElement,
    Pixels, SharedString, Styled, StyledText, relative, TextRun, UnderlineStyle, Window, anchored, canvas, deferred, div, point,
    prelude::*, px, svg,
};
use gpui_component::tooltip::Tooltip;

use super::ClayView;
use super::assets::icon;
use super::output::hsla;
use crate::native_gui::core::client::World;
use crate::native_gui::core::style::{Rgb, StyleOptions, StyledLine, style_line};
use crate::theme::{ThemeColor, ThemeColors};

/// CSS `line-height: normal` for the two fonts: their own ascent + descent (hhea, which
/// is also their typo metrics) over the em. GPUI's default is taller.
pub const SANS_LH: f32 = 1.364;
pub const MONO_LH: f32 = 1.32;
/// `--status-height`, borders included.
pub const BAR_HEIGHT: f32 = 26.0;
/// The More badge's reds (style.css's own, not theme colours).
const MORE_LABEL: Rgb = [0xcc, 0x1a, 0x0e];
const MORE_COUNT: Rgb = [0xa1, 0x0b, 0x00];
/// PAUSED's `var(--theme-muted-fg, #888)` names no theme colour, so it is always #888.
const PAUSED_BG: Rgb = [0x88, 0x88, 0x88];
/// The ACT count's border, `rgba(184, 152, 48, 0.3)`.
const ACT_BORDER: Rgb = [184, 152, 48];
/// The font slider's range (index.html's `<input type="range" min="7" max="20">`).
pub const FONT_MIN: f32 = 7.0;
pub const FONT_MAX: f32 = 20.0;
/// The slider's track width and its thumb's diameter.
const SLIDER_W: f32 = 60.0;
const THUMB: f32 = 12.0;

/// The theme as the chrome uses it: style.css's variables, resolved.
#[derive(Clone, Debug, PartialEq)]
pub struct Tokens {
    pub bar: Hsla,
    pub surface: Hsla,
    pub elevated: Hsla,
    pub hover: Hsla,
    pub fg: Hsla,
    pub fg2: Hsla,
    pub muted: Hsla,
    pub dim: Hsla,
    pub accent: Hsla,
    pub highlight: Hsla,
    pub success: Hsla,
    pub error: Hsla,
    pub activity: Hsla,
    pub border: Hsla,
    pub border_subtle: Hsla,
    /// The icon bar's tiles: at rest, and hovered or on.
    pub tile_rest: Hsla,
    pub tile_hover_bg: Hsla,
    pub tile_hover_fg: Hsla,
    /// What the light theme's rules apply to (`body.theme-light`).
    pub light: bool,
}

fn rgb(c: &ThemeColor) -> Rgb {
    [c.r, c.g, c.b]
}

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    std::array::from_fn(|i| (a[i] as f32 * (1.0 - t) + b[i] as f32 * t).round() as u8)
}

pub fn alpha(c: Rgb, a: f32) -> Hsla {
    let mut h = hsla(c);
    h.a = a;
    h
}

impl Tokens {
    pub fn new(t: &ThemeColors, light: bool) -> Self {
        // `color-mix(in srgb, <bar> 100%, white 15%)` (35% in the light theme): the weights
        // are scaled to sum to 100%.
        let white = if light { 35.0 } else { 15.0 };
        let tile_rest = mix(rgb(&t.status_bar_bg), [255, 255, 255], white / (100.0 + white));
        let (hover_bg, hover_fg) = if light { (&t.accent, &t.bg) } else { (&t.highlight, &t.bg_surface) };
        Tokens {
            bar: hsla(rgb(&t.status_bar_bg)),
            surface: hsla(rgb(&t.bg_surface)),
            elevated: hsla(rgb(&t.bg_elevated)),
            hover: hsla(rgb(&t.bg_hover)),
            fg: hsla(rgb(&t.fg)),
            fg2: hsla(rgb(&t.fg_secondary)),
            muted: hsla(rgb(&t.fg_muted)),
            dim: hsla(rgb(&t.fg_dim)),
            accent: hsla(rgb(&t.accent)),
            highlight: hsla(rgb(&t.highlight)),
            success: hsla(rgb(&t.success)),
            error: hsla(rgb(&t.error)),
            activity: hsla(rgb(&t.activity_bg)),
            border: hsla(rgb(&t.border_medium)),
            border_subtle: hsla(rgb(&t.border_subtle)),
            tile_rest: hsla(tile_rest),
            tile_hover_bg: hsla(rgb(hover_bg)),
            tile_hover_fg: hsla(rgb(hover_fg)),
            light,
        }
    }

    /// A world's connection dot: on, still trying (amber), or off.
    pub fn dot(&self, world: &World) -> Hsla {
        if world.connected {
            self.success
        } else if reconnecting(world) {
            self.highlight
        } else {
            self.error
        }
    }
}

/// app.js `isWorldReconnecting`: off, but auto-reconnect is set and it was connected.
fn reconnecting(w: &World) -> bool {
    !w.connected && w.was_connected && w.settings.as_ref().is_some_and(|s| auto_reconnect_secs(&s.auto_reconnect_secs) > 0)
}

/// app.js `parseAutoReconnectSecs`: the last all-digit part of "30" / "web,30".
fn auto_reconnect_secs(raw: &str) -> u32 {
    raw.split(',').filter_map(|p| p.trim().parse::<u32>().ok()).next_back().unwrap_or(0)
}

/// app.js `formatCount`, without the padding (the browser collapses it).
pub fn format_count(n: usize) -> String {
    if n >= 1_000_000 {
        "Alot".into()
    } else if n >= 10_000 {
        format!("{}K", (n / 1000).min(999))
    } else {
        n.to_string()
    }
}

/// The 7px dot with its glow.
pub fn status_dot(color: Hsla) -> gpui::Div {
    let mut glow = color;
    glow.a = 0.35;
    div().size(px(7.0)).flex_shrink_0().rounded_full().bg(color).shadow(vec![BoxShadow {
        color: glow,
        offset: point(px(0.0), px(0.0)),
        blur_radius: px(5.0),
        spread_radius: px(0.0),
        inset: false,
    }])
}

/// The two menus the status bar opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuKind {
    Main,
    Worlds,
}

/// The hamburger menu (index.html `#menu-dropdown`): (action, label, shortcut). `None` is
/// a separator.
const MAIN_MENU: &[Option<(&str, &str, &str)>] = &[
    Some(("help", "Help", "F1")),
    Some(("world-selector", "World Selector", "/worlds")),
    Some(("actions", "Actions", "/actions")),
    None,
    Some(("setup", "Settings", "/setup")),
    Some(("import", "Import Settings", "/import")),
    None,
    Some(("toggle-tags", "Toggle Tags", "F2")),
    Some(("filter", "Find", "F4")),
    Some(("search", "Search History", "F5")),
    None,
    Some(("change-password", "Change Password", "")),
    Some(("reload", "Reload", "Ctrl+R")),
    Some(("new-window", "New Window", "")),
    Some(("resync", "Resync", "")),
    None,
    Some(("logout", "Logout", "")),
];

/// The chrome's own state.
pub struct Chrome {
    pub tokens: Tokens,
    pub menu: Option<MenuKind>,
    /// A menu the current mouse press closed (by pressing outside it): that press's
    /// click on the menu's own button must not open it again.
    pub closed_by_press: Option<MenuKind>,
    pub slider: Rc<Cell<Bounds<Pixels>>>,
    pub slider_drag: bool,
    pub clock: String,
    pub stats_collapsed: bool,
    /// The TF status rows' width in columns, as last measured.
    pub tf_cols: Rc<Cell<usize>>,
    pub tabs_scroll: gpui::ScrollHandle,
    pub shortcuts_scroll: gpui::ScrollHandle,
}

impl Chrome {
    pub fn new(tokens: Tokens) -> Self {
        Chrome {
            tokens,
            menu: None,
            closed_by_press: None,
            slider: Rc::new(Cell::new(Bounds::default())),
            slider_drag: false,
            clock: String::new(),
            stats_collapsed: crate::native_gui::core::store::pref("stats_collapsed").as_deref() == Some("1"),
            tf_cols: Rc::new(Cell::new(80)),
            tabs_scroll: gpui::ScrollHandle::new(),
            shortcuts_scroll: gpui::ScrollHandle::new(),
        }
    }
}

/// The clock as the status bar shows it: Clay's 12-hour H:MM, or TF's %clock_format.
pub fn clock_text(format: &str) -> String {
    if format.is_empty() { crate::util::get_current_time_12hr() } else { crate::util::format_clock(format) }
}

/// A badge half: text on a colour, as the status bar's two-part badges are drawn.
fn badge(text: impl Into<SharedString>, fg: Hsla, bg: Hsla, pad: (f32, f32), corners: (bool, bool)) -> gpui::Div {
    let mut d = div().pl(px(pad.0)).pr(px(pad.1)).py(px(2.0)).text_color(fg).bg(bg).child(text.into());
    let r = px(3.0);
    if corners.0 {
        d = d.rounded_tl(r).rounded_bl(r);
    }
    if corners.1 {
        d = d.rounded_tr(r).rounded_br(r);
    }
    d
}

/// A styled line (escape codes already applied) as text runs, in one font.
pub fn styled_text(line: &StyledLine, family: &SharedString) -> StyledText {
    let runs = line
        .runs
        .iter()
        .map(|r| {
            let mut font = gpui::font(family.clone());
            if r.bold {
                font.weight = FontWeight::BOLD;
            }
            if r.italic {
                font.style = gpui::FontStyle::Italic;
            }
            TextRun {
                len: r.len,
                font,
                color: hsla(r.fg),
                background_color: r.bg.map(hsla),
                underline: r.underline.then(|| UnderlineStyle { thickness: px(1.0), color: None, wavy: false }),
                strikethrough: None,
            }
        })
        .collect();
    StyledText::new(line.text.clone()).with_runs(runs)
}

impl ClayView {
    /// The status bar (index.html `#status-bar`), with the menus it opens.
    pub(super) fn status_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = self.chrome.tokens.clone();
        let mono = self.fonts.primary.clone();
        let sans = self.fonts.ui.clone();
        let core = &self.session.core;
        let world = core.current_world();
        let tf = world.and_then(|w| w.tf_status.clone());

        let mut row = div().flex().flex_1().items_center().px(px(12.0)).gap(px(10.0)).text_size(px(11.5)).font_family(sans.clone()).line_height(relative(SANS_LH));

        // The menu button.
        let menu_open = self.chrome.menu == Some(MenuKind::Main);
        row = row.child(
            // The menu goes first: an absolute element's origin is where it would have
            // been in flow, and before the button that is the button's top-left corner.
            div()
                .relative()
                .when(menu_open, |d| d.child(self.main_menu(cx)))
                .child(self.icon_button("menu-btn", "menu", "Menu", cx).on_click(cx.listener(|this, _, _w, cx| this.toggle_menu(MenuKind::Main, cx)))),
        );

        if let Some(view) = tf {
            // TF's status area in place of the bar's own indicators; the controls stay.
            let cols = self.chrome.tf_cols.get().max(1);
            let opts = StyleOptions { discord_emoji: false, ..Default::default() };
            let mut rows = div().flex().flex_col().flex_1().min_w_0().overflow_hidden().font_family(mono.clone()).line_height(relative(1.2)).text_color(t.fg);
            for cells in &view.rows {
                let laid = crate::tf::status::layout_row(cells, cols, &view.pad, &view.attr);
                let styled = style_line(&laid, &self.palette, &opts);
                rows = rows.child(div().overflow_hidden().whitespace_nowrap().child(styled_text(&styled, &mono)));
            }
            // How many columns the strip holds, in its own font: measured after layout,
            // and a frame again if that changed.
            let cell = self.chrome.tf_cols.clone();
            let font = gpui::font(mono.clone());
            rows = rows.child(
                canvas(
                    move |bounds, window, _cx| {
                        let ts = window.text_system();
                        let id = ts.resolve_font(&font);
                        let ch = ts.ch_advance(id, px(11.5)).map(f32::from).unwrap_or(7.0).max(1.0);
                        let n = (f32::from(bounds.size.width) / ch).floor().max(1.0) as usize;
                        if n != cell.get() {
                            cell.set(n);
                            window.refresh();
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            );
            row = row.child(div().relative().flex_1().min_w_0().flex().py(px(2.0)).child(rows));
        } else {
            // The world: its dot and name, and the [g]/[N] markers; click for the world menu.
            if let Some(w) = world.filter(|w| !w.name.is_empty() && w.was_connected) {
                let mut name = w.name.clone();
                if w.gmcp_user_enabled {
                    name.push_str(" [g]");
                }
                if let Some(n) = self.kbnum {
                    name.push_str(&format!(" [{n}]"));
                }
                let open = self.chrome.menu == Some(MenuKind::Worlds);
                row = row.child(
                    div()
                        .relative()
                        .when(open, |d| d.child(self.world_menu(cx)))
                        .child(
                            div()
                                .id("status-world")
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .min_w(px(76.0))
                                .cursor_pointer()
                                .child(status_dot(t.dot(w)))
                                .child(div().font_weight(FontWeight::BOLD).text_color(t.fg).whitespace_nowrap().child(name))
                                .on_click(cx.listener(|this, _, _w, cx| this.toggle_menu(MenuKind::Worlds, cx))),
                        ),
                );
            }

            if let Some(note) = self.connection_note() {
                row = row.child(div().text_color(t.muted).whitespace_nowrap().overflow_hidden().child(note));
            }

            // More (held output) or History (scrolled back): click releases a screenful.
            let (rows_below, at_bottom) = {
                let s = self.scroll.borrow();
                (s.rows_below, s.at_bottom())
            };
            let pending = self.pending_total();
            let more = if !at_bottom {
                Some(("History", rows_below))
            } else if (self.more.paused && self.more.held() > 0) || world.is_some_and(|w| w.pending_count > 0) {
                Some(("More", pending))
            } else {
                None
            };
            if let Some((label, n)) = more {
                row = row.child(
                    div()
                        .id("status-more")
                        .flex()
                        .items_center()
                        .flex_shrink_0()
                        .cursor_pointer()
                        .font_family(mono.clone()).line_height(relative(MONO_LH))
                        .text_size(px(11.0))
                        .font_weight(FontWeight::BOLD)
                        .whitespace_nowrap()
                        .child(badge(label, gpui::white(), hsla(MORE_LABEL), (8.0, 6.0), (true, false)))
                        .child(badge(format_count(n), gpui::white(), hsla(MORE_COUNT), (6.0, 8.0), (false, true)))
                        .on_click(cx.listener(|this, _, _w, cx| this.release_screenful(cx))),
                );
            }

            // Activity elsewhere: click goes to the next world.
            if core.activity > 0 {
                let cur = core.current;
                let names: Vec<&str> = core
                    .worlds
                    .iter()
                    .enumerate()
                    .filter(|(i, w)| *i != cur && (w.unseen > 0 || w.pending_count > 0))
                    .map(|(_, w)| w.name.as_str())
                    .collect();
                let tip: SharedString = if names.is_empty() { "".into() } else { format!("Unseen: {}", names.join(", ")).into() };
                let mut count = badge(core.activity.to_string(), gpui::black(), t.highlight, (5.0, 6.0), (false, true));
                count = count.border_1().border_l_0().border_color(alpha(ACT_BORDER, 0.3));
                let mut act = div()
                    .id("status-activity")
                    .flex()
                    .items_center()
                    .flex_shrink_0()
                    .cursor_pointer()
                    .font_family(mono.clone()).line_height(relative(MONO_LH))
                    .text_size(px(10.5))
                    .font_weight(FontWeight::BOLD)
                    .child(badge("ACT", gpui::black(), t.activity, (6.0, 5.0), (true, false)).border_1().border_color(gpui::transparent_black()))
                    .child(count)
                    .on_click(cx.listener(|this, _, w, cx| this.run_action("world_next", w, cx)));
                if !tip.is_empty() {
                    act = act.tooltip(move |w, cx| Tooltip::new(tip.clone()).build(w, cx));
                }
                row = row.child(act);
            }

            // This session paused (/remote --pause).
            if core.paused {
                row = row.child(
                    div()
                        .flex_shrink_0()
                        .font_family(mono.clone()).line_height(relative(MONO_LH))
                        .text_size(px(10.5))
                        .font_weight(FontWeight::BOLD)
                        .opacity(0.85)
                        .child(badge("PAUSED", gpui::white(), hsla(PAUSED_BG), (7.0, 7.0), (true, true))),
                );
            }
        }

        // The spacer (TF's strip takes the room itself).
        if !self.tf_hidden_indicators() {
            row = row.child(div().flex_1());
        }

        // The notes icon, when this world has notes.
        if world.and_then(|w| w.settings.as_ref()).is_some_and(|s| s.has_notes) {
            row = row.child(self.icon_button("status-note", "note", "Notes", cx).on_click(cx.listener(|this, _, _w, _cx| this.open_surface("notes"))));
        }

        row = row.child(self.font_slider(cx));

        // TF's area hides the scrollback badge and the clock too.
        if !self.tf_hidden_indicators() {
            if let Some(pct) = self.session.core.backfill_progress() {
                row = row.child(
                    div()
                        .flex()
                        .items_center()
                        .flex_shrink_0()
                        .font_family(mono.clone()).line_height(relative(MONO_LH))
                        .text_size(px(10.5))
                        .font_weight(FontWeight::BOLD)
                        .whitespace_nowrap()
                        .child(badge("Scrollback", gpui::black(), t.accent, (6.0, 5.0), (true, false)).opacity(0.85).border_1().border_color(gpui::transparent_black()))
                        .child(badge(format!("{pct}%"), gpui::white(), t.surface, (5.0, 6.0), (false, true)).border_1().border_l_0().border_color(t.accent)),
                );
            }
            row = row.child(
                div().flex_shrink_0().font_family(mono.clone()).line_height(relative(MONO_LH)).text_size(px(11.0)).font_weight(FontWeight::MEDIUM).text_color(t.fg).whitespace_nowrap().child(self.chrome.clock.clone()),
            );
        }
        let _ = window;

        div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .min_h(px(BAR_HEIGHT))
            .bg(t.bar)
            .child(div().h(px(1.0)).bg(t.border))
            .child(div().flex().min_h(px(BAR_HEIGHT - 2.0)).child(row))
            .child(div().h(px(1.0)).bg(t.border_subtle))
            .into_any_element()
    }

    /// Whether TF's status area is standing in for the bar's indicators.
    fn tf_hidden_indicators(&self) -> bool {
        self.session.core.current_world().is_some_and(|w| w.tf_status.is_some())
    }

    /// A `.status-btn`: a 15px icon that lights up in the accent colour on hover.
    fn icon_button(&self, id: &'static str, name: &str, title: &'static str, _cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let t = &self.chrome.tokens;
        let (accent, muted, hover) = (t.accent, t.muted, t.hover);
        div()
            .id(id)
            .group(id)
            .flex()
            .items_center()
            .flex_shrink_0()
            .px(px(5.0))
            .py(px(3.0))
            .rounded(px(4.0))
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .child(svg().path(icon(name)).size(px(15.0)).text_color(muted).group_hover(id, move |s| s.text_color(accent)))
            .tooltip(move |w, cx| Tooltip::new(title).build(w, cx))
    }

    pub(super) fn toggle_menu(&mut self, which: MenuKind, cx: &mut Context<Self>) {
        if self.chrome.closed_by_press.take() == Some(which) {
            // This press closed it (it was outside the menu): leave it closed.
        } else {
            self.chrome.menu = if self.chrome.menu == Some(which) { None } else { Some(which) };
        }
        cx.notify();
    }

    /// A dropdown in `.menu-dropdown`'s style, opening upward from where it's placed.
    fn dropdown(&self, which: MenuKind, items: Vec<AnyElement>, cx: &mut Context<Self>) -> AnyElement {
        let t = &self.chrome.tokens;
        let panel = div()
            .id(match which {
                MenuKind::Main => "menu-dropdown",
                MenuKind::Worlds => "world-menu-dropdown",
            })
            .occlude()
            .flex()
            .flex_col()
            .min_w(px(190.0))
            .p(px(5.0))
            .bg(t.elevated)
            .border_1()
            .border_color(t.border)
            .rounded(px(8.0))
            .shadow(vec![
                BoxShadow { color: alpha([0, 0, 0], 0.3), offset: point(px(0.0), px(2.0)), blur_radius: px(8.0), spread_radius: px(0.0), inset: false },
                BoxShadow { color: alpha([0, 0, 0], 0.18), offset: point(px(0.0), px(12.0)), blur_radius: px(32.0), spread_radius: px(0.0), inset: false },
            ])
            .font_family(self.fonts.ui.clone()).line_height(relative(SANS_LH))
            .text_size(px(12.5))
            .children(items)
            .on_mouse_down_out(cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                if this.chrome.menu == Some(which) {
                    this.chrome.menu = None;
                    this.chrome.closed_by_press = Some(which);
                    cx.notify();
                }
            }));
        deferred(anchored().anchor(Anchor::BottomLeft).offset(point(px(0.0), px(-4.0))).snap_to_window().child(panel))
            .with_priority(1)
            .into_any_element()
    }

    /// One `.menu-item`.
    fn menu_item(&self, id: SharedString, lead: Option<AnyElement>, label: String, shortcut: String, selected: bool) -> gpui::Stateful<gpui::Div> {
        let t = &self.chrome.tokens;
        let (hover, fg) = (t.hover, t.fg);
        let mut item = div()
            .id(id)
            .flex()
            .items_center()
            .gap(px(8.0))
            .px(px(11.0))
            .py(px(7.0))
            .rounded(px(4.0))
            .cursor_pointer()
            .text_color(if selected { t.accent } else { t.fg2 })
            .font_weight(if selected { FontWeight::BOLD } else { FontWeight::MEDIUM })
            .hover(move |s| s.bg(hover).text_color(fg));
        if let Some(lead) = lead {
            item = item.child(lead);
        }
        item = item.child(label);
        if !shortcut.is_empty() {
            item = item.child(
                div().ml_auto().text_color(t.dim).text_size(px(10.5)).font_family(self.fonts.primary.clone()).line_height(relative(MONO_LH)).font_weight(FontWeight::NORMAL).child(shortcut),
            );
        }
        item
    }

    fn main_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let multiuser = self.session.core.is_multiuser();
        let t = &self.chrome.tokens;
        let mut items = Vec::new();
        let mut last_sep = true;
        for entry in MAIN_MENU {
            match entry {
                None if !last_sep => {
                    items.push(div().h(px(1.0)).mx(px(8.0)).my(px(4.0)).bg(t.border).into_any_element());
                    last_sep = true;
                }
                None => {}
                Some((action, label, shortcut)) => {
                    if matches!(*action, "change-password" | "logout") && !multiuser {
                        continue;
                    }
                    let action: &'static str = action;
                    items.push(
                        self.menu_item(format!("menu-{action}").into(), None, label.to_string(), shortcut.to_string(), false)
                            .on_click(cx.listener(move |this, _, w, cx| this.menu_action(action, w, cx)))
                            .into_any_element(),
                    );
                    last_sep = false;
                }
            }
        }
        if last_sep {
            items.pop();
        }
        self.dropdown(MenuKind::Main, items, cx)
    }

    /// The world-switch menu: connected worlds, and disconnected ones with unseen output.
    fn world_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let core = &self.session.core;
        let list: Vec<(usize, &World)> = core.worlds.iter().enumerate().filter(|(_, w)| w.connected || w.unseen > 0).collect();
        let hide_current = list.len() > 5;
        let mut items = Vec::new();
        for (i, w) in list {
            if hide_current && i == core.current {
                continue;
            }
            let unseen = if w.unseen > 0 { format!("[{} unseen]", w.unseen) } else { String::new() };
            items.push(
                self.menu_item(format!("world-{i}").into(), Some(status_dot(self.chrome.tokens.dot(w)).into_any_element()), w.name.clone(), unseen, i == core.current)
                    .on_click(cx.listener(move |this, _, _w, cx| {
                        this.chrome.menu = None;
                        this.switch_to(i, cx);
                    }))
                    .into_any_element(),
            );
        }
        self.dropdown(MenuKind::Worlds, items, cx)
    }

    pub(super) fn menu_action(&mut self, action: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.chrome.menu = None;
        match action {
            "toggle-tags" => self.run_action("toggle_tags", window, cx),
            "reload" | "resync" => {
                let fx = self.session.core.restart();
                let rest = self.session.apply(fx);
                self.after(rest, cx);
            }
            "filter" => self.toggle_filter(window, cx),
            "search" => self.toggle_search(window, cx),
            "logout" => {
                let fx = self.session.core.logout();
                let rest = self.session.apply(fx);
                self.after(rest, cx);
            }
            other => self.open_surface(other),
        }
        cx.notify();
    }

    /// A window or dialog the native GUI doesn't have yet.
    pub(super) fn open_surface(&mut self, name: &str) {
        crate::debug_log(crate::is_debug_enabled(), &format!("native-gui: {name} is not available yet"));
    }

    /// Show world `index` (a menu, a tab).
    pub(super) fn switch_to(&mut self, index: usize, cx: &mut Context<Self>) {
        let fx = self.session.core.switch_world(index, Instant::now());
        let rest = self.session.apply(fx);
        self.after(rest, cx);
        cx.notify();
    }

    /// The font size: "A" makes it smaller, the number bigger, the slider sets it.
    fn font_slider(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = &self.chrome.tokens;
        let size = self.font_size.round().clamp(FONT_MIN, FONT_MAX);
        let (accent, muted, border) = (t.accent, t.muted, t.border);
        let label = |id: &'static str, text: String, sz: f32, weight: FontWeight, title: &'static str| {
            div()
                .id(id)
                .font_family(self.fonts.primary.clone()).line_height(relative(MONO_LH))
                .text_size(px(sz))
                .font_weight(weight)
                .text_color(muted)
                .cursor_pointer()
                .hover(move |s| s.text_color(accent))
                .child(text)
                .tooltip(move |w, cx| Tooltip::new(title).build(w, cx))
        };
        let bounds = self.chrome.slider.clone();
        let frac = (size - FONT_MIN) / (FONT_MAX - FONT_MIN);
        let track = canvas(
            move |b, _, _| bounds.set(b),
            move |b, _, window, _cx| {
                let mid = b.origin.y + b.size.height / 2.0;
                window.paint_quad(gpui::fill(Bounds::new(point(b.origin.x, mid - px(2.0)), gpui::size(px(SLIDER_W), px(4.0))), border).corner_radii(px(2.0)));
                let x = b.origin.x + px(THUMB / 2.0 + frac * (SLIDER_W - THUMB));
                let thumb = Bounds::new(point(x - px(THUMB / 2.0), mid - px(THUMB / 2.0)), gpui::size(px(THUMB), px(THUMB)));
                window.paint_quad(gpui::quad(thumb, px(THUMB / 2.0), muted, px(1.5), border, gpui::BorderStyle::Solid));
            },
        )
        .w(px(SLIDER_W))
        .h(px(THUMB));
        div()
            .flex()
            .items_center()
            .gap(px(5.0))
            .flex_shrink_0()
            .child(label("font-dec", "A".into(), 9.0, FontWeight::SEMIBOLD, "Decrease font size").on_click(cx.listener(move |this, _, _w, cx| this.set_font_size(size - 1.0, cx))))
            .child(
                div()
                    .id("font-slider")
                    .cursor_pointer()
                    .child(track)
                    .tooltip(|w, cx| Tooltip::new("Font size").build(w, cx))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, ev: &MouseDownEvent, _w, cx| {
                            this.chrome.slider_drag = true;
                            this.slide_to(ev.position.x, cx);
                        }),
                    ),
            )
            .child(
                label("font-inc", format!("{size}"), 9.5, FontWeight::MEDIUM, "Increase font size")
                    .min_w(px(18.0))
                    .flex()
                    .justify_center()
                    .on_click(cx.listener(move |this, _, _w, cx| this.set_font_size(size + 1.0, cx))),
            )
            .into_any_element()
    }

    /// The slider dragged (or pressed) at window x.
    pub(super) fn slide_to(&mut self, x: Pixels, cx: &mut Context<Self>) {
        let b = self.chrome.slider.get();
        let frac = (f32::from(x - b.origin.x) - THUMB / 2.0) / (SLIDER_W - THUMB);
        let size = FONT_MIN + (frac.clamp(0.0, 1.0) * (FONT_MAX - FONT_MIN)).round();
        self.set_font_size(size, cx);
    }

    /// A new output font size: shown at once, and saved to the server (it's the
    /// `web_font_size_desktop` setting every desktop client shares).
    pub(super) fn set_font_size(&mut self, size: f32, cx: &mut Context<Self>) {
        let size = size.round().clamp(FONT_MIN, FONT_MAX);
        if size == self.font_size {
            return;
        }
        self.font_size = size;
        let follow = self.scroll.borrow().follow;
        if follow {
            self.scroll.borrow_mut().follow_bottom();
        }
        if let Some(s) = self.session.core.settings.as_mut() {
            s.web_font_size_desktop = size;
            let msg = crate::native_gui::core::client::settings_update(s);
            let fx = self.session.send(msg);
            self.after(fx, cx);
        }
        cx.notify();
    }
}
