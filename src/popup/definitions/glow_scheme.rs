//! "New Background - Custom": the window of glow colour schemes the Setup popup's New
//! Indicator opens (user request 2026-10-28: "a pop up window with 10 or so options to choose
//! from ... show a 4 letter word with the fading"). Each row draws `newbg::SAMPLE_WORD` over
//! its scheme's four cells, fading to this console's black or white (`newbg::console_target`),
//! beside the scheme's name. Pushed over /setup by Enter on the Custom entry; Enter (or OK)
//! puts the scheme on the Setup select, Esc and Cancel leave it as it was. The web client
//! (app.js `showGlowSchemeDialog`) and the native GUI (`dialogs::small::GlowScheme`) offer the
//! same list the same way.

use crate::popup::{Button, ButtonId, Field, FieldId, FieldKind, ListItem, ListItemStyle, PopupDefinition, PopupId, PopupLayout, PopupManager, PopupState};

pub const GLOW_SCHEME_FIELD_LIST: FieldId = FieldId(1);

// Not 1 and 2: a popup with both of those is taken for a confirm dialog (`is_confirm`).
pub const GLOW_SCHEME_BTN_OK: ButtonId = ButtonId(10);
pub const GLOW_SCHEME_BTN_CANCEL: ButtonId = ButtonId(11);

/// The picker, on scheme `current` (an index into `newbg::SCHEMES`), its swatches fading to
/// `to` (the console's black or white).
pub fn create_glow_scheme_popup(current: usize, to: crate::reveal::Rgb) -> PopupDefinition {
    let items: Vec<ListItem> = crate::newbg::SCHEMES
        .iter()
        .map(|s| ListItem {
            id: s.key.to_string(),
            columns: vec![crate::newbg::SAMPLE_WORD.to_string(), s.label.to_string()],
            style: ListItemStyle { swatch: Some(crate::newbg::swatch(s.start, to).map(|c| c.array())), ..Default::default() },
        })
        .collect();
    let height = items.len();
    let mut list = FieldKind::list(items, height);
    if let FieldKind::List { selected_index, .. } = &mut list {
        *selected_index = current.min(crate::newbg::SCHEMES.len() - 1);
    }
    PopupDefinition::new(PopupId("glow_scheme"), "New Background - Custom")
        .with_field(Field::new(GLOW_SCHEME_FIELD_LIST, "", list))
        .with_button(Button::new(GLOW_SCHEME_BTN_OK, "OK").primary().with_shortcut('O'))
        .with_button(Button::new(GLOW_SCHEME_BTN_CANCEL, "Cancel").with_shortcut('C'))
        .with_layout(PopupLayout {
            label_width: 0,
            min_width: 34,
            max_width_percent: 60,
            center_horizontal: true,
            center_vertical: true,
            modal: true,
            buttons_right_align: true,
            blank_line_before_list: false,
            tab_buttons_only: false,
            anchor_bottom_left: false,
            anchor_x: 0,
        })
}

/// Push the picker over the Setup popup (the current one), on the scheme its New Indicator
/// select shows.
pub fn open_glow_scheme_picker(popups: &mut PopupManager, theme: crate::encoding::Theme) {
    let shown = popups.current().and_then(|s| s.get_selected(super::setup::SETUP_FIELD_NEW_LINE_INDICATOR)).unwrap_or("").to_string();
    let current = match crate::NewIndicator::from_name(&shown) {
        crate::NewIndicator::Custom(i) => i as usize,
        _ => 0,
    };
    popups.push(create_glow_scheme_popup(current, crate::newbg::console_target(theme)));
    if let Some(state) = popups.current_mut() {
        state.select_field(GLOW_SCHEME_FIELD_LIST);
    }
}

/// The value the highlighted row makes (`custom:<key>`).
pub fn picked_value(state: &PopupState) -> Option<String> {
    let key = state.get_selected_list_item()?.id.clone();
    let i = crate::newbg::scheme_index(&key)?;
    Some(crate::NewIndicator::Custom(i as u8).name())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_picker_lists_every_scheme_with_its_swatch_and_opens_on_the_current_one() {
        let def = create_glow_scheme_popup(3, crate::reveal::Rgb(0, 0, 0));
        let state = PopupState::new(def);
        let Some(FieldKind::List { items, selected_index, .. }) = state.field(GLOW_SCHEME_FIELD_LIST).map(|f| &f.kind) else { panic!("a list") };
        assert_eq!(items.len(), crate::newbg::SCHEMES.len());
        assert_eq!(*selected_index, 3);
        for (item, s) in items.iter().zip(crate::newbg::SCHEMES.iter()) {
            assert_eq!(item.columns, vec!["Clay".to_string(), s.label.to_string()]);
            assert_eq!(item.style.swatch, Some(crate::newbg::swatch(s.start, crate::reveal::Rgb(0, 0, 0)).map(|c| c.array())));
        }
        assert_eq!(picked_value(&state).as_deref(), Some("custom:indigo"));
    }
}
