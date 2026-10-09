//! Clay's theme on gpui-component's widgets (text fields, selects and their menus,
//! sliders, switches, lists, tables, buttons, tooltips): their colours from the 42 theme
//! colours, their fonts from the web client's.

use gpui::App;
use gpui_component::Theme;

use super::fonts::Fonts;
use super::output::hsla;
use crate::theme::{ThemeColor, ThemeColors};

fn c(t: &ThemeColor) -> gpui::Hsla {
    hsla([t.r, t.g, t.b])
}

pub fn apply(t: &ThemeColors, fonts: &Fonts, cx: &mut App) {
    let theme = Theme::global_mut(cx);
    theme.font_family = fonts.ui.clone();
    theme.mono_font_family = fonts.primary.clone();
    let k = &mut theme.colors;
    k.background = c(&t.bg);
    k.foreground = c(&t.fg);
    k.border = c(&t.border_medium);
    k.input = c(&t.border_medium);
    k.ring = c(&t.accent);
    k.caret = c(&t.fg);
    k.selection = c(&t.selection_bg);
    k.primary = c(&t.accent);
    k.primary_hover = c(&t.accent);
    k.primary_active = c(&t.accent_dim);
    k.primary_foreground = c(&t.fg);
    k.secondary = c(&t.bg_surface);
    k.secondary_hover = c(&t.bg_hover);
    k.secondary_active = c(&t.bg_hover);
    k.secondary_foreground = c(&t.fg);
    k.accent = c(&t.bg_hover);
    k.accent_foreground = c(&t.fg);
    k.muted = c(&t.bg_surface);
    k.muted_foreground = c(&t.fg_muted);
    k.popover = c(&t.bg_elevated);
    k.popover_foreground = c(&t.fg);
    k.link = c(&t.link);
    k.link_hover = c(&t.link);
    k.button = c(&t.bg_surface);
    k.button_hover = c(&t.bg_hover);
    k.button_active = c(&t.bg_hover);
    k.button_foreground = c(&t.fg);
    k.button_primary = c(&t.accent);
    k.button_primary_hover = c(&t.accent);
    k.button_primary_active = c(&t.accent_dim);
    k.button_primary_foreground = c(&t.fg);
    k.danger = c(&t.error);
    k.danger_foreground = c(&t.fg);
    k.success = c(&t.success);
    k.overlay = gpui::Hsla { a: 0.6, ..gpui::black() };
    // The dialogs' and windows' widgets (phase 4's design pass): lists and selects'
    // menus, sliders, switches, scrollbars, tabs and tables, in the theme's own colours.
    let error = c(&t.error);
    k.danger_hover = gpui::Hsla { l: (error.l + 0.08).min(1.0), ..error };
    k.danger_active = c(&t.error);
    k.warning = c(&t.highlight);
    k.warning_foreground = c(&t.bg);
    k.info = c(&t.accent);
    k.info_foreground = c(&t.fg);
    k.list = c(&t.bg_elevated);
    k.list_even = c(&t.bg_elevated);
    k.list_head = c(&t.bg_surface);
    k.list_hover = c(&t.bg_hover);
    k.list_active = c(&t.selection_bg);
    k.list_active_border = c(&t.accent);
    k.slider_bar = c(&t.accent);
    k.slider_thumb = c(&t.fg);
    k.switch = c(&t.border_medium);
    k.switch_thumb = c(&t.fg);
    k.scrollbar = gpui::transparent_black();
    k.scrollbar_thumb = c(&t.fg_muted);
    k.scrollbar_thumb_hover = c(&t.fg_secondary);
    k.tab_bar = c(&t.bg_surface);
    k.tab = c(&t.bg_surface);
    k.tab_active = c(&t.bg_elevated);
    k.tab_foreground = c(&t.fg_muted);
    k.tab_active_foreground = c(&t.fg);
    k.table = c(&t.bg);
    k.table_even = c(&t.bg);
    k.table_head = c(&t.bg_surface);
    k.table_head_foreground = c(&t.accent);
    k.table_hover = c(&t.bg_hover);
    k.table_active = c(&t.selection_bg);
    k.table_active_border = c(&t.accent);
    k.table_row_border = c(&t.border_subtle);
    k.drag_border = c(&t.accent);
    k.drop_target = gpui::Hsla { a: 0.25, ..c(&t.accent) };
    k.title_bar = c(&t.status_bar_bg);
    k.status_bar = c(&t.status_bar_bg);
    // The widgets read tokens derived from the colours when a theme is built: derive
    // them again, or they keep gpui-component's own theme.
    theme.tokens = gpui_component::theme::ThemeTokens::from(&theme.colors);
}
