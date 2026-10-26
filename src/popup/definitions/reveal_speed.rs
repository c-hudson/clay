//! Custom Text Reveal Speed: the number the Setup popup's Text Reveal "Custom" entry asks
//! for (user request 2026-10-25: "a custom option ... the user will be prompted with a pop
//! up to type in the value"), and New Only's the same way (2026-10-26). Pushed over /setup
//! by Enter on either entry of its Text Reveal, or by a Save while one has no number yet -
//! cycling past them, and moving off the field, never asks (`setup::text_reveal_needs_number`);
//! OK puts `custom:<n>` / `new:<n>` on the Setup select (relabelled "Custom (n)" /
//! "New Only (n)") and finishes a Save that asked, Cancel and Esc leave the select as it was.
//! The web client (app.js `showRevealSpeedDialog`) and the native GUI
//! (`dialogs::small::RevealSpeed`) ask the same way.

use crate::popup::{Button, ButtonId, Field, FieldId, FieldKind, PopupDefinition, PopupId, PopupLayout, PopupManager, PopupState};

pub const REVEAL_SPEED_FIELD_CPS: FieldId = FieldId(1);

// Not 1 and 2: a popup with both of those is taken for a confirm dialog (`is_confirm`).
pub const REVEAL_SPEED_BTN_OK: ButtonId = ButtonId(10);
pub const REVEAL_SPEED_BTN_CANCEL: ButtonId = ButtonId(11);

/// custom_data: the Setup select's value before the entry was picked, which Cancel restores.
pub const REVEAL_SPEED_PREVIOUS: &str = "reveal_speed_previous";
/// custom_data: the entry that asked (`TextReveal::CUSTOM` or `NEW_ONLY`), what OK makes.
pub const REVEAL_SPEED_ENTRY: &str = "reveal_speed_entry";
/// custom_data: Save asked for the number, so OK saves the Setup popup too.
pub const REVEAL_SPEED_THEN_SAVE: &str = "reveal_speed_then_save";

/// The prompt for `entry`, prefilled with its current speed when there is one.
pub fn create_reveal_speed_popup(entry: &str, current: Option<u32>, previous: &str) -> PopupDefinition {
    let mut def = PopupDefinition::new(PopupId("reveal_speed"), crate::TextReveal::speed_title(entry))
        .with_field(Field::new(
            REVEAL_SPEED_FIELD_CPS,
            "Chars a second",
            FieldKind::text(current.map(|cps| cps.to_string()).unwrap_or_default()),
        ))
        .with_button(Button::new(REVEAL_SPEED_BTN_OK, "OK").primary().with_shortcut('O'))
        .with_button(Button::new(REVEAL_SPEED_BTN_CANCEL, "Cancel").with_shortcut('C'))
        .with_layout(PopupLayout {
            label_width: 16,
            min_width: 46,
            max_width_percent: 60,
            center_horizontal: true,
            center_vertical: true,
            modal: true,
            buttons_right_align: true,
            blank_line_before_list: false,
            tab_buttons_only: false,
            anchor_bottom_left: false,
            anchor_x: 0,
        });
    def.custom_data.insert(REVEAL_SPEED_PREVIOUS.to_string(), previous.to_string());
    def.custom_data.insert(REVEAL_SPEED_ENTRY.to_string(), entry.to_string());
    def
}

/// Push the prompt over the Setup popup (which is the current one) for the entry its
/// select is on, typing straight into its field: prefilled with that entry's speed,
/// remembering `previous` for Cancel, and whether a Save asked (`then_save`).
pub fn open_reveal_speed_prompt(popups: &mut PopupManager, previous: &str, then_save: bool) {
    let picked = popups.current().and_then(|s| s.get_selected(super::setup::SETUP_FIELD_TEXT_REVEAL)).map(str::to_string);
    let Some(entry) = picked.as_deref().and_then(crate::TextReveal::speed_entry) else { return };
    let current = picked.as_deref().and_then(|v| crate::TextReveal::from_name(v).typed_cps());
    let mut def = create_reveal_speed_popup(entry, current, previous);
    if then_save {
        def.custom_data.insert(REVEAL_SPEED_THEN_SAVE.to_string(), "1".to_string());
    }
    popups.push(def);
    if let Some(state) = popups.current_mut() {
        state.start_edit();
    }
}

/// The typed speed, or the complaint to show.
pub fn typed_speed(state: &PopupState) -> Result<u32, &'static str> {
    crate::TextReveal::parse_custom(state.get_text(REVEAL_SPEED_FIELD_CPS).unwrap_or("")).ok_or(crate::TextReveal::CUSTOM_PROBLEM)
}

/// What OK makes of the typed speed: the asking entry's value (`custom:<n>`, `new:<n>`).
pub fn typed_value(state: &PopupState) -> Result<crate::TextReveal, &'static str> {
    let entry = state.definition.custom_data.get(REVEAL_SPEED_ENTRY).map_or(crate::TextReveal::CUSTOM, String::as_str);
    typed_speed(state).map(|cps| crate::TextReveal::with_speed(entry, cps))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prompt_is_prefilled_and_remembers_the_previous_choice() {
        let def = create_reveal_speed_popup("custom", Some(150), "fast");
        assert_eq!(def.title, "Custom Text Reveal Speed");
        assert_eq!(def.custom_data.get(REVEAL_SPEED_PREVIOUS).map(String::as_str), Some("fast"));
        let state = PopupState::new(def);
        assert_eq!(state.get_text(REVEAL_SPEED_FIELD_CPS), Some("150"));
        assert_eq!(typed_speed(&state), Ok(150));
        assert_eq!(typed_value(&state), Ok(crate::TextReveal::Custom(150)));
        // New Only asks the same way, under its own title, for its own kind of value.
        let state = PopupState::new(create_reveal_speed_popup("new", Some(80), "custom:150"));
        assert_eq!(state.definition.title, "New Only Text Reveal Speed");
        assert_eq!(typed_value(&state), Ok(crate::TextReveal::NewOnly(80)));
        let mut state = PopupState::new(create_reveal_speed_popup("custom", None, "off"));
        assert_eq!(state.get_text(REVEAL_SPEED_FIELD_CPS), Some(""));
        assert_eq!(typed_speed(&state), Err(crate::TextReveal::CUSTOM_PROBLEM));
        for bad in ["0", "10001", "abc", "12.5", "-3"] {
            state.set_text(REVEAL_SPEED_FIELD_CPS, bad.to_string());
            assert_eq!(typed_speed(&state), Err(crate::TextReveal::CUSTOM_PROBLEM), "{bad}");
        }
        state.set_text(REVEAL_SPEED_FIELD_CPS, " 10000 ".to_string());
        assert_eq!(typed_speed(&state), Ok(10000));
    }

    /// Picked over /setup: the prompt is on top, typing goes into its field, and the
    /// Setup popup is back underneath afterwards with its select as it was.
    #[test]
    fn the_prompt_opens_over_setup_prefilled_from_its_select() {
        let mut popups = PopupManager::new();
        popups.open(super::super::setup::create_setup_popup_for_test("custom:75"));
        open_reveal_speed_prompt(&mut popups, "normal", false);
        let state = popups.current().unwrap();
        assert_eq!(state.definition.id, PopupId("reveal_speed"));
        assert!(state.editing);
        assert_eq!(state.get_text(REVEAL_SPEED_FIELD_CPS), Some("75"));
        popups.close();
        assert_eq!(popups.current().unwrap().definition.id, PopupId("setup"));
        assert_eq!(popups.current().unwrap().get_selected(super::super::setup::SETUP_FIELD_TEXT_REVEAL), Some("custom:75"));
        // On New Only, the New Only prompt, prefilled from its value.
        let mut popups = PopupManager::new();
        popups.open(super::super::setup::create_setup_popup_for_test("new:30"));
        open_reveal_speed_prompt(&mut popups, "fast", false);
        let state = popups.current().unwrap();
        assert_eq!(state.definition.title, "New Only Text Reveal Speed");
        assert_eq!(state.get_text(REVEAL_SPEED_FIELD_CPS), Some("30"));
        // On a preset, no prompt.
        let mut popups = PopupManager::new();
        popups.open(super::super::setup::create_setup_popup_for_test("fast"));
        open_reveal_speed_prompt(&mut popups, "off", false);
        assert_eq!(popups.current().unwrap().definition.id, PopupId("setup"));
    }
}
