//! Modify Auth Key popup — copy, regenerate, or delete the device auth key.
//!
//! Opened from the /web popup's "Modify Key" button; the Auth Key field there is
//! read-only, so this is the only place the key can be changed. Regen/Delete take
//! effect immediately (persisted and broadcast to every connected web/GUI client
//! via App::handle_ws_key_request/handle_ws_key_revoke) — they don't wait for the
//! /web popup's own Save button.

use crate::popup::{Button, ButtonId, Field, FieldId, FieldKind, PopupDefinition, PopupId, PopupLayout};

pub const MODIFY_KEY_FIELD_KEY: FieldId = FieldId(1);

pub const MODIFY_KEY_BTN_COPY: ButtonId = ButtonId(1);
pub const MODIFY_KEY_BTN_REGEN: ButtonId = ButtonId(2);
pub const MODIFY_KEY_BTN_DELETE: ButtonId = ButtonId(3);
pub const MODIFY_KEY_BTN_CLOSE: ButtonId = ButtonId(4);

/// Create the Modify Key popup, showing the current key (read-only) and offering
/// Copy / Regen / Delete actions.
pub fn create_modify_key_popup(auth_key: &str) -> PopupDefinition {
    PopupDefinition::new(PopupId("modify_key"), "Modify Auth Key")
        .with_field(
            Field::new(
                MODIFY_KEY_FIELD_KEY,
                "Auth Key",
                FieldKind::text(display_key(auth_key)),
            )
            .disabled(),
        )
        .with_button(Button::new(MODIFY_KEY_BTN_COPY, "Copy").with_shortcut('P'))
        .with_button(Button::new(MODIFY_KEY_BTN_REGEN, "Regen").with_shortcut('R'))
        .with_button(Button::new(MODIFY_KEY_BTN_DELETE, "Delete").danger().with_shortcut('D'))
        .with_button(Button::new(MODIFY_KEY_BTN_CLOSE, "Close").primary().with_shortcut('L'))
        .with_layout(PopupLayout {
            label_width: 10,
            min_width: 50,
            max_width_percent: 90,
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

/// Update the popup's displayed key after a Regen/Delete (the field is purely
/// presentational — Copy/Regen/Delete act on `app.settings.websocket_auth_key`
/// directly, not on this field's text).
pub fn set_displayed_key(state: &mut crate::popup::PopupState, auth_key: &str) {
    state.set_text(MODIFY_KEY_FIELD_KEY, display_key(auth_key).to_string());
}

fn display_key(auth_key: &str) -> &str {
    if auth_key.is_empty() { "(none)" } else { auth_key }
}
