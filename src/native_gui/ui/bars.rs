//! The other bars around the output, as the web client draws them: the icon bar above it
//! (`#icon-bar`: the built-in tiles and the Actions marked as shortcuts), the world-tabs
//! ribbon (`#tabs-ribbon`) and the MUD's status panel (`#stats-panel`).

use gpui::{
    AnyElement, Context, FontWeight, IntoElement, ParentElement, Styled, Visibility, div, point, prelude::*, px, relative, svg,
};
use gpui_component::tooltip::Tooltip;

use super::ClayView;
use super::chrome::{MONO_LH, SANS_LH};
use super::assets::icon;
use crate::stats::{StatEntry, StatValue};
use crate::websocket::WsMessage;

/// How far an arrow scrolls the tabs or the shortcuts.
const ARROW_STEP: f32 = 120.0;

/// The built-in tiles: (menu action, icon, label, title).
const TILES: &[(&str, &str, &str, &str)] = &[
    ("world-selector", "worlds", "Worlds", "World Selector"),
    ("actions", "actions", "Actions", "Actions"),
    ("setup", "settings", "Settings", "Settings"),
    ("filter", "find", "Find", "Find"),
    ("toggle-tags", "tags", "Tags", "Toggle Tags"),
];

/// Laid out but not drawn (`visibility: hidden`), so the row doesn't shift.
fn invisible<E: Styled>(mut e: E) -> E {
    e.style().visibility = Some(Visibility::Hidden);
    e
}

/// app.js `formatStatLabel`: HP/MP/... upper-cased, other keys title-cased.
pub fn stat_label(key: &str) -> String {
    let lower = key.to_lowercase();
    if ["hp", "mp", "mv", "xp", "lvl", "lv"].contains(&lower.as_str()) {
        return key.to_uppercase();
    }
    key.replace(['.', '_'], " ")
        .split(' ')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            let first = c.next().map(|f| f.to_uppercase().to_string()).unwrap_or_default();
            first + &c.as_str().to_lowercase()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn stat_value(v: &StatValue) -> String {
    match v {
        StatValue::Plain(s) => s.clone(),
        StatValue::Gauge { current, maximum } => format!("{current} / {maximum}"),
    }
}

/// How full a gauge is, 0 to 1.
pub fn gauge_fill(current: &str, maximum: &str) -> f32 {
    match (current.trim().parse::<f32>(), maximum.trim().parse::<f32>()) {
        (Ok(c), Ok(m)) if c.is_finite() && m.is_finite() && m > 0.0 => (c / m).clamp(0.0, 1.0),
        _ => 0.0,
    }
}

/// Scroll a strip `step` pixels along (negative is left), within its range.
fn scroll_by(handle: &gpui::ScrollHandle, step: f32) {
    let max = f32::from(handle.max_offset().x);
    let now = f32::from(handle.offset().x);
    let x = (now - step).clamp(-max, 0.0);
    handle.set_offset(point(px(x), handle.offset().y));
}

impl ClayView {
    /// Whether the icon bar shows: the `icon_bar` setting is "app_tablet" (this is the
    /// desktop app) or "all".
    fn icon_bar_on(&self) -> bool {
        self.session.core.settings.as_ref().is_some_and(|s| s.icon_bar != "none")
    }

    /// A tile: an icon over a label, lifted off the bar, lit when hovered or on.
    fn tile(&self, id: gpui::SharedString, icon_name: &str, label: String, title: String, on: bool) -> gpui::Stateful<gpui::Div> {
        let t = &self.chrome.tokens;
        let (bg, fg) = if on { (t.tile_hover_bg, t.tile_hover_fg) } else { (t.tile_rest, t.fg) };
        let (hover_bg, hover_fg) = (t.tile_hover_bg, t.tile_hover_fg);
        div()
            .id(id.clone())
            .group(id.clone())
            .flex()
            .flex_col()
            .items_center()
            .flex_shrink_0()
            .gap(px(3.0))
            .w(px(53.0))
            .pt(px(6.0))
            .px(px(3.0))
            .pb(px(5.0))
            .rounded(px(8.0))
            .bg(bg)
            .cursor_pointer()
            .hover(move |s| s.bg(hover_bg))
            .child(svg().path(icon(icon_name)).size(px(24.0)).text_color(fg).group_hover(id.clone(), move |s| s.text_color(hover_fg)))
            .child(
                div()
                    .w_full()
                    .flex()
                    .justify_center()
                    .text_center()
                    .font_family(self.fonts.ui.clone()).line_height(relative(SANS_LH))
                    .text_size(px(9.5))
                    .font_weight(FontWeight::BOLD)
                    .line_height(relative(1.1))
                    .text_color(fg)
                    .group_hover(id, move |s| s.text_color(hover_fg))
                    .child(label),
            )
            .tooltip(move |w, cx| Tooltip::new(title.clone()).build(w, cx))
    }

    /// A round ‹ or › arrow that scrolls the shortcuts.
    fn round_arrow(&self, id: &'static str, icon_name: &str, title: &'static str, shown: bool) -> gpui::Stateful<gpui::Div> {
        let t = &self.chrome.tokens;
        let (hover_bg, hover_fg, muted) = (t.tile_hover_bg, t.tile_hover_fg, t.muted);
        let a = div()
            .id(id)
            .group(id)
            .flex()
            .items_center()
            .justify_center()
            .flex_shrink_0()
            .size(px(17.0))
            .rounded_full()
            .cursor_pointer()
            .hover(move |s| s.bg(hover_bg))
            .child(svg().path(icon(icon_name)).size(px(10.0)).text_color(muted).group_hover(id, move |s| s.text_color(hover_fg)))
            .tooltip(move |w, cx| Tooltip::new(title).build(w, cx));
        if shown { a } else { invisible(a) }
    }

    pub(super) fn icon_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.icon_bar_on() {
            return None;
        }
        let t = &self.chrome.tokens;
        let show_tags = self.show_tags;
        let mut row = div().flex().items_center().px(px(7.0)).py(px(5.0)).gap(px(2.0));
        for (action, icon_name, label, title) in TILES {
            let action: &'static str = action;
            let on = action == "toggle-tags" && show_tags;
            row = row.child(
                self.tile(format!("tile-{action}").into(), icon_name, label.to_string(), title.to_string(), on)
                    .on_click(cx.listener(move |this, _, w, cx| this.menu_action(action, w, cx))),
            );
        }
        row = row.child(div().flex_1());
        let shortcuts: Vec<(usize, String)> = self
            .session
            .core
            .actions
            .iter()
            .enumerate()
            .filter(|(_, a)| a.enabled && a.gui_shortcut)
            .map(|(i, a)| (i, a.name.clone()))
            .collect();
        if !shortcuts.is_empty() {
            let handle = self.chrome.shortcuts_scroll.clone();
            let overflowing = f32::from(handle.max_offset().x) > 0.5;
            let mut strip = div().id("icon-bar-shortcuts").flex().items_center().gap(px(2.0)).min_w_0().max_w(px(221.0)).overflow_x_scroll().track_scroll(&handle);
            for (i, name) in shortcuts {
                let title = name.clone();
                strip = strip.child(self.tile(format!("shortcut-{i}").into(), "star", name.clone(), title, false).on_click(cx.listener(
                    move |this, _, _w, cx| {
                        let command = format!("/{name}");
                        let world_index = this.session.core.current;
                        this.more_mut().caught_up();
                        let fx = this.session.send(WsMessage::SendCommand { world_index, command });
                        this.after(fx, cx);
                    },
                )));
            }
            let (left, right) = (handle.clone(), handle);
            row = row
                .child(div().w(px(1.0)).self_stretch().mx(px(3.0)).my(px(7.0)).flex_shrink_0().bg(t.border))
                .child(self.round_arrow("icon-bar-left", "arrow-left", "Previous shortcuts", overflowing).on_click(move |_, w, _| {
                    scroll_by(&left, -ARROW_STEP);
                    w.refresh();
                }))
                .child(strip)
                .child(self.round_arrow("icon-bar-right", "arrow-right", "More shortcuts", overflowing).on_click(move |_, w, _| {
                    scroll_by(&right, ARROW_STEP);
                    w.refresh();
                }));
        }
        Some(
            div()
                .flex()
                .flex_col()
                .flex_shrink_0()
                .bg(t.bar)
                .child(div().h(px(1.0)).bg(t.border))
                .child(row)
                .child(div().h(px(1.0)).bg(t.border_subtle))
                .into_any_element(),
        )
    }

    /// Where the tabs go: "top", "bottom", or nowhere.
    pub(super) fn tabs_mode(&self) -> &str {
        self.session.core.settings.as_ref().map_or("none", |s| s.tabs.as_str())
    }

    /// The world tabs: one per connected world.
    pub(super) fn tabs_ribbon(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !matches!(self.tabs_mode(), "top" | "bottom") {
            return None;
        }
        let core = &self.session.core;
        let connected: Vec<(usize, String)> = core.worlds.iter().enumerate().filter(|(_, w)| w.connected).map(|(i, w)| (i, w.name.clone())).collect();
        if connected.is_empty() {
            return None;
        }
        let t = &self.chrome.tokens;
        let handle = self.chrome.tabs_scroll.clone();
        let overflowing = f32::from(handle.max_offset().x) > 0.5;
        let mut tabs = div().id("tabs-ribbon-tabs").flex().flex_1().items_center().gap(px(4.0)).min_w_0().h_full().overflow_x_scroll().track_scroll(&handle);
        for (i, name) in connected {
            let active = i == core.current;
            let (hover, fg) = (t.hover, t.fg);
            // Every tab has the tab shape; the current one is lifted onto the elevated
            // colour with the accent as its top edge, the others sit on a faint lift with
            // the border as theirs - so the ribbon needs no line of its own above them.
            let mut tab = div()
                .id(format!("tab-{i}"))
                .flex()
                .items_center()
                .flex_shrink_0()
                .px(px(12.0))
                .pt(px(4.0))
                .pb(px(3.0))
                .rounded_tl(px(6.0))
                .rounded_tr(px(6.0))
                .border_t_2()
                .whitespace_nowrap()
                .cursor_pointer()
                .font_family(self.fonts.ui.clone()).line_height(relative(SANS_LH))
                .text_size(px(11.5))
                .child(name);
            tab = if active {
                tab.bg(t.elevated).border_color(t.accent).text_color(t.fg).font_weight(FontWeight::BOLD)
            } else {
                tab.bg(gpui::Hsla { a: 0.07, ..t.fg }).border_color(t.border).text_color(t.fg2).hover(move |s| s.bg(hover).text_color(fg))
            };
            tabs = tabs.child(tab.on_click(cx.listener(move |this, _, _w, cx| this.switch_to(i, cx))));
        }
        let arrow = |id: &'static str, glyph: &'static str, title: &'static str, step: f32| {
            let (hover, fg) = (t.hover, t.fg);
            let h = handle.clone();
            let a = div()
                .id(id)
                .flex()
                .items_center()
                .justify_center()
                .flex_shrink_0()
                .w(px(18.0))
                .h_full()
                .rounded(px(4.0))
                .text_size(px(10.0))
                .text_color(t.fg2)
                .cursor_pointer()
                .hover(move |s| s.bg(hover).text_color(fg))
                .child(glyph)
                .tooltip(move |w, cx| Tooltip::new(title).build(w, cx))
                .on_click(move |_, w, _| {
                    scroll_by(&h, step);
                    w.refresh();
                });
            if overflowing { a } else { invisible(a) }
        };
        // The tabs sit on the ribbon's top edge (no top line: each tab's own edge is the
        // line, and only where a tab is); the ribbon keeps its bottom line.
        let row = div()
            .flex()
            .items_start()
            .h(px(super::chrome::BAR_HEIGHT - 1.0))
            .px(px(4.0))
            .gap(px(4.0))
            .child(arrow("tabs-ribbon-left", "◀", "Scroll left", -ARROW_STEP))
            .child(tabs)
            .child(arrow("tabs-ribbon-right", "▶", "Scroll right", ARROW_STEP));
        Some(
            div()
                .flex()
                .flex_col()
                .flex_shrink_0()
                .bg(t.bar)
                .child(row)
                .child(div().h(px(1.0)).bg(t.border_subtle))
                .into_any_element(),
        )
    }

    /// The MUD's status display (GMCP/MSDP vitals), when the world has any.
    pub(super) fn stats_panel(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let world = self.session.core.current_world()?;
        if world.stats.is_empty() {
            return None;
        }
        let t = &self.chrome.tokens;
        let collapsed = self.chrome.stats_collapsed;
        let header = div()
            .id("stats-panel-header")
            .flex()
            .items_center()
            .gap(px(6.0))
            .px(px(10.0))
            .py(px(2.0))
            .cursor_pointer()
            .text_color(t.fg2)
            .child(div().font_weight(FontWeight::BOLD).text_size(px(10.0)).child("STATUS"))
            .child(div().ml_auto().text_size(px(10.0)).child(if collapsed { "▸" } else { "▾" }))
            .tooltip(move |w, cx| Tooltip::new(if collapsed { "Expand status panel" } else { "Collapse status panel" }).build(w, cx))
            .on_click(cx.listener(|this, _, _w, cx| {
                this.chrome.stats_collapsed = !this.chrome.stats_collapsed;
                crate::native_gui::core::store::set_pref("stats_collapsed", if this.chrome.stats_collapsed { "1" } else { "0" });
                cx.notify();
            }));
        let mut panel = div().flex().flex_col().flex_shrink_0().font_family(self.fonts.ui.clone()).line_height(relative(SANS_LH)).text_size(px(11.0)).child(header);
        if !collapsed {
            let mut body = div().flex().flex_wrap().items_center().gap_x(px(16.0)).gap_y(px(3.0)).px(px(10.0)).pt(px(2.0)).pb(px(6.0));
            for entry in &world.stats {
                body = body.child(self.stat_row(entry));
            }
            panel = panel.child(body);
        }
        Some(
            div()
                .flex()
                .flex_col()
                .flex_shrink_0()
                .bg(t.bar)
                .child(div().h(px(1.0)).bg(t.border))
                .child(panel)
                .child(div().h(px(1.0)).bg(t.border_subtle))
                .into_any_element(),
        )
    }

    fn stat_row(&self, entry: &StatEntry) -> AnyElement {
        let t = &self.chrome.tokens;
        let mut row = div().flex().items_center().gap(px(6.0)).min_w(px(110.0)).child(div().text_color(t.fg2).whitespace_nowrap().child(stat_label(&entry.key)));
        if let StatValue::Gauge { current, maximum } = &entry.value {
            let fill = gauge_fill(current, maximum);
            row = row.child(
                div()
                    .w(px(60.0))
                    .h(px(6.0))
                    .flex_shrink_0()
                    .rounded(px(3.0))
                    .bg(t.elevated)
                    .border_1()
                    .border_color(t.border)
                    .overflow_hidden()
                    .child(div().h_full().w(relative(fill)).bg(t.accent)),
            );
        }
        row.child(div().text_color(t.fg).font_family(self.fonts.primary.clone()).line_height(relative(MONO_LH)).whitespace_nowrap().child(stat_value(&entry.value))).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_labels_and_gauges_read_like_the_web_client() {
        assert_eq!(stat_label("hp"), "HP");
        assert_eq!(stat_label("Mv"), "MV");
        assert_eq!(stat_label("char.vitals_max"), "Char Vitals Max");
        assert_eq!(stat_label("GOLD"), "Gold");
        assert_eq!(gauge_fill("50", "200"), 0.25);
        assert_eq!(gauge_fill("300", "200"), 1.0);
        assert_eq!(gauge_fill("x", "200"), 0.0);
        assert_eq!(gauge_fill("5", "0"), 0.0);
    }
}
