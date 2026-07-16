//! /import popup definition — collects host[:port], password, and an optional auth-key
//! for the master console TUI's in-process import driver.
//!
//! See plan i-d-like-to-make-snuggly-rain.md, step 8. Fields are unmasked-vs-masked the
//! same way world_editor.rs treats a world's own connection password (unmasked) versus
//! slack_token/discord_token (masked, FieldKind::password): this dialog's password/auth-key
//! are one-time login-style credentials for the *target* instance, not a value being
//! redisplayed for later editing, so they're masked like any other secret entry field.

use crate::popup::{Button, ButtonId, Field, FieldId, FieldKind, PopupDefinition, PopupId, PopupLayout};

pub const IMPORT_FIELD_ADDR: FieldId = FieldId(1);
pub const IMPORT_FIELD_PASSWORD: FieldId = FieldId(2);
pub const IMPORT_FIELD_AUTH_KEY: FieldId = FieldId(3);

pub const IMPORT_BTN_GO: ButtonId = ButtonId(1);
pub const IMPORT_BTN_CANCEL: ButtonId = ButtonId(2);

/// Create the /import popup, with the host[:port] prefilled from the command line if given.
pub fn create_import_popup(prefill_addr: &str) -> PopupDefinition {
    PopupDefinition::new(PopupId("import"), "Import Settings")
        .with_field(Field::new(
            IMPORT_FIELD_ADDR,
            "Host[:port]",
            FieldKind::text(prefill_addr),
        ))
        .with_field(Field::new(
            IMPORT_FIELD_PASSWORD,
            "Password",
            FieldKind::password(""),
        ))
        .with_field(Field::new(
            IMPORT_FIELD_AUTH_KEY,
            "Auth key (instead of password)",
            FieldKind::password(""),
        ))
        .with_button(Button::new(IMPORT_BTN_GO, "Import").primary())
        .with_button(Button::new(IMPORT_BTN_CANCEL, "Cancel"))
        .with_layout(PopupLayout {
            label_width: 30,
            min_width: 50,
            max_width_percent: 70,
            center_horizontal: true,
            center_vertical: true,
            modal: true,
            buttons_right_align: false,
            blank_line_before_list: false,
            tab_buttons_only: false,
            anchor_bottom_left: false,
            anchor_x: 0,
        })
        .with_help(vec![
            "Pull worlds, theme, and keybindings from another Clay instance.".to_string(),
            "Remote values win on conflicts; everything else you have locally is kept.".to_string(),
            "Enter a password OR an auth key, not both.".to_string(),
        ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::popup::PopupState;

    #[test]
    fn test_import_popup_creation() {
        let def = create_import_popup("example.com:9000");
        let state = PopupState::new(def);
        assert_eq!(state.definition.id, PopupId("import"));
        assert_eq!(state.get_text(IMPORT_FIELD_ADDR), Some("example.com:9000"));
        assert_eq!(state.get_text(IMPORT_FIELD_PASSWORD), Some(""));
        // with_help() prepends a "?" help button, so 2 real buttons + 1 help button.
        assert_eq!(state.definition.buttons.len(), 3);
    }
}
