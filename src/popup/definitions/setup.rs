//! Setup (global settings) popup definition
//!
//! Allows editing global application settings.

use crate::popup::{
    Button, ButtonId, Field, FieldId, FieldKind, PopupDefinition, PopupId, PopupLayout,
    SelectOption,
};

// Field IDs
pub const SETUP_FIELD_MORE_MODE: FieldId = FieldId(1);
pub const SETUP_FIELD_SPELL_CHECK: FieldId = FieldId(2);
pub const SETUP_FIELD_TEMP_CONVERT: FieldId = FieldId(3);
pub const SETUP_FIELD_WORLD_SWITCHING: FieldId = FieldId(4);
pub const SETUP_FIELD_DEBUG: FieldId = FieldId(5);
// Note: show_tags is now a temporary in-memory setting controlled by F2 or /tag command
pub const SETUP_FIELD_INPUT_HEIGHT: FieldId = FieldId(7);
pub const SETUP_FIELD_GUI_THEME: FieldId = FieldId(8);
pub const SETUP_FIELD_TLS_PROXY: FieldId = FieldId(9);
pub const SETUP_FIELD_DICTIONARY: FieldId = FieldId(10);
pub const SETUP_FIELD_EDITOR_SIDE: FieldId = FieldId(11);
pub const SETUP_FIELD_MOUSE: FieldId = FieldId(12);
pub const SETUP_FIELD_ZWJ: FieldId = FieldId(13);
pub const SETUP_FIELD_ANSI_MUSIC: FieldId = FieldId(14);

// Button IDs
pub const SETUP_BTN_SAVE: ButtonId = ButtonId(1);
pub const SETUP_BTN_CANCEL: ButtonId = ButtonId(2);

/// World switching mode options
pub fn world_switching_options() -> Vec<SelectOption> {
    vec![
        SelectOption::new("unseen_first", "Unseen First"),
        SelectOption::new("alphabetical", "Alphabetical"),
    ]
}

/// Theme options
pub fn theme_options() -> Vec<SelectOption> {
    vec![
        SelectOption::new("dark", "Dark"),
        SelectOption::new("light", "Light"),
    ]
}

/// Editor side options
pub fn editor_side_options() -> Vec<SelectOption> {
    vec![
        SelectOption::new("left", "Left"),
        SelectOption::new("right", "Right"),
    ]
}

/// Create the setup popup definition with current values
#[allow(clippy::too_many_arguments)]
pub fn create_setup_popup(
    more_mode: bool,
    spell_check: bool,
    temp_convert: bool,
    world_switching: &str,
    debug: bool,
    input_height: i64,
    gui_theme: &str,
    tls_proxy: bool,
    dictionary_path: &str,
    editor_side: &str,
    mouse_enabled: bool,
    zwj_enabled: bool,
    ansi_music: bool,
) -> PopupDefinition {
    let world_switching_idx = if world_switching == "alphabetical" { 1 } else { 0 };
    let gui_theme_idx = if gui_theme == "light" { 1 } else { 0 };
    let editor_side_idx = if editor_side == "right" { 1 } else { 0 };

    PopupDefinition::new(PopupId("setup"), "Setup")
        .with_field(Field::new(
            SETUP_FIELD_MORE_MODE,
            "More Mode",
            FieldKind::toggle(more_mode),
        ))
        .with_field(Field::new(
            SETUP_FIELD_SPELL_CHECK,
            "Spell Check",
            FieldKind::toggle(spell_check),
        ))
        .with_field(Field::new(
            SETUP_FIELD_TEMP_CONVERT,
            "Temp Convert",
            FieldKind::toggle(temp_convert),
        ))
        .with_field(Field::new(
            SETUP_FIELD_WORLD_SWITCHING,
            "World Switching",
            FieldKind::select(world_switching_options(), world_switching_idx),
        ))
        .with_field(Field::new(
            SETUP_FIELD_DEBUG,
            "Debug",
            FieldKind::toggle(debug),
        ))
        .with_field(Field::new(
            SETUP_FIELD_INPUT_HEIGHT,
            "Input Height",
            FieldKind::number(input_height),
        ))
        .with_field(Field::new(
            SETUP_FIELD_GUI_THEME,
            "GUI Theme",
            FieldKind::select(theme_options(), gui_theme_idx),
        ))
        .with_field(Field::new(
            SETUP_FIELD_TLS_PROXY,
            "TLS Proxy",
            FieldKind::toggle(tls_proxy),
        ))
        .with_field(Field::new(
            SETUP_FIELD_DICTIONARY,
            "Dictionary",
            FieldKind::text(dictionary_path),
        ))
        .with_field(Field::new(
            SETUP_FIELD_EDITOR_SIDE,
            "Editor Side",
            FieldKind::select(editor_side_options(), editor_side_idx),
        ))
        .with_field(Field::new(
            SETUP_FIELD_MOUSE,
            "Console Mouse",
            FieldKind::toggle(mouse_enabled),
        ))
        .with_field(Field::new(
            SETUP_FIELD_ZWJ,
            "ZWJ Sequence",
            FieldKind::toggle(zwj_enabled),
        ))
        .with_field(Field::new(
            SETUP_FIELD_ANSI_MUSIC,
            "ANSI Music",
            FieldKind::toggle(ansi_music),
        ))
        .with_button(Button::new(SETUP_BTN_CANCEL, "Cancel").with_shortcut('C'))
        .with_button(Button::new(SETUP_BTN_SAVE, "Save").primary().with_shortcut('S'))
        .with_layout(PopupLayout {
            label_width: 17,
            min_width: 40,
            max_width_percent: 60,
            center_horizontal: true,
            center_vertical: true,
            modal: true,
            buttons_right_align: true,
            blank_line_before_list: false,
            tab_buttons_only: false,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::popup::PopupState;

    #[test]
    fn test_setup_popup_creation() {
        let def = create_setup_popup(
            true, true, false, "unseen_first",
            false, 3, "dark", false, "", "left", false, false, true,
        );
        let state = PopupState::new(def);

        assert_eq!(state.definition.id, PopupId("setup"));
        assert_eq!(state.definition.title, "Setup");
        assert_eq!(state.definition.fields.len(), 13);
        assert_eq!(state.definition.buttons.len(), 2);
    }

    #[test]
    fn test_setup_popup_values() {
        let def = create_setup_popup(
            true, false, true, "alphabetical",
            true, 5, "light", true, "/custom/dict", "left", true, true, true,
        );
        let state = PopupState::new(def);

        assert_eq!(state.get_bool(SETUP_FIELD_MORE_MODE), Some(true));
        assert_eq!(state.get_bool(SETUP_FIELD_SPELL_CHECK), Some(false));
        assert_eq!(state.get_bool(SETUP_FIELD_TEMP_CONVERT), Some(true));
        assert_eq!(state.get_selected(SETUP_FIELD_WORLD_SWITCHING), Some("alphabetical"));
        assert_eq!(state.get_bool(SETUP_FIELD_DEBUG), Some(true));
        assert_eq!(state.get_number(SETUP_FIELD_INPUT_HEIGHT), Some(5));
        assert_eq!(state.get_selected(SETUP_FIELD_GUI_THEME), Some("light"));
        assert_eq!(state.get_bool(SETUP_FIELD_TLS_PROXY), Some(true));
        assert_eq!(state.get_bool(SETUP_FIELD_MOUSE), Some(true));
        assert_eq!(state.get_bool(SETUP_FIELD_ZWJ), Some(true));
    }
}
