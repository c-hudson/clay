//! Clay's theme on gpui-component's widgets (the text fields of the login screen and the
//! Find and Search popups, buttons, tooltips): its colours from the 42 theme colours, its
//! fonts from the web client's. The full design pass, for the dialogs and windows of
//! phase 4, builds on this.

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
}
