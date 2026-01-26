//! Web settings popup definition
//!
//! Allows editing HTTP/HTTPS and WebSocket server settings.

use crate::popup::{
    Button, ButtonId, Field, FieldId, FieldKind, PopupDefinition, PopupId, PopupLayout,
    SelectOption,
};

// Field IDs
pub const WEB_FIELD_PROTOCOL: FieldId = FieldId(1);
pub const WEB_FIELD_HTTP_ENABLED: FieldId = FieldId(2);
pub const WEB_FIELD_HTTP_PORT: FieldId = FieldId(3);
pub const WEB_FIELD_WS_ENABLED: FieldId = FieldId(4);
pub const WEB_FIELD_WS_PORT: FieldId = FieldId(5);
pub const WEB_FIELD_WS_PASSWORD: FieldId = FieldId(6);
pub const WEB_FIELD_WS_ALLOW_LIST: FieldId = FieldId(7);
pub const WEB_FIELD_WS_CERT_FILE: FieldId = FieldId(8);
pub const WEB_FIELD_WS_KEY_FILE: FieldId = FieldId(9);

// Button IDs
pub const WEB_BTN_SAVE: ButtonId = ButtonId(1);
pub const WEB_BTN_CANCEL: ButtonId = ButtonId(2);

/// Protocol options
pub fn protocol_options() -> Vec<SelectOption> {
    vec![
        SelectOption::new("nonsecure", "Non-Secure"),
        SelectOption::new("secure", "Secure"),
    ]
}

/// Create the web settings popup definition with current values
#[allow(clippy::too_many_arguments)]
pub fn create_web_popup(
    web_secure: bool,
    http_enabled: bool,
    http_port: &str,
    ws_enabled: bool,
    ws_port: &str,
    ws_password: &str,
    ws_allow_list: &str,
    ws_cert_file: &str,
    ws_key_file: &str,
) -> PopupDefinition {
    let protocol_idx = if web_secure { 1 } else { 0 };

    let mut def = PopupDefinition::new(PopupId("web"), "Web Settings")
        .with_field(Field::new(
            WEB_FIELD_PROTOCOL,
            "Protocol",
            FieldKind::select(protocol_options(), protocol_idx),
        ))
        .with_field(Field::new(
            WEB_FIELD_HTTP_ENABLED,
            "HTTP Enabled",
            FieldKind::toggle(http_enabled),
        ))
        .with_field(Field::new(
            WEB_FIELD_HTTP_PORT,
            "HTTP Port",
            FieldKind::text(http_port),
        ))
        .with_field(Field::new(
            WEB_FIELD_WS_ENABLED,
            "WS Enabled",
            FieldKind::toggle(ws_enabled),
        ))
        .with_field(Field::new(
            WEB_FIELD_WS_PORT,
            "WS Port",
            FieldKind::text(ws_port),
        ))
        .with_field(Field::new(
            WEB_FIELD_WS_PASSWORD,
            "WS Password",
            FieldKind::text(ws_password),
        ))
        .with_field(Field::new(
            WEB_FIELD_WS_ALLOW_LIST,
            "WS Allow List",
            FieldKind::text(ws_allow_list),
        ))
        .with_field(Field::new(
            WEB_FIELD_WS_CERT_FILE,
            "TLS Cert File",
            FieldKind::text(ws_cert_file),
        ))
        .with_field(Field::new(
            WEB_FIELD_WS_KEY_FILE,
            "TLS Key File",
            FieldKind::text(ws_key_file),
        ))
        .with_button(Button::new(WEB_BTN_CANCEL, "Cancel").with_shortcut('C'))
        .with_button(Button::new(WEB_BTN_SAVE, "Save").primary().with_shortcut('S'))
        .with_layout(PopupLayout {
            label_width: 15,
            min_width: 50,
            max_width_percent: 90,
            center_horizontal: true,
            center_vertical: true,
            modal: true,
            buttons_right_align: true,
            blank_line_before_list: false,
        });

    // Hide TLS fields if not secure
    if !web_secure {
        if let Some(field) = def.get_field_mut(WEB_FIELD_WS_CERT_FILE) {
            field.visible = false;
        }
        if let Some(field) = def.get_field_mut(WEB_FIELD_WS_KEY_FILE) {
            field.visible = false;
        }
    }

    def
}

/// Update visibility of TLS fields based on protocol selection
pub fn update_tls_visibility(state: &mut crate::popup::PopupState) {
    let is_secure = state.get_selected(WEB_FIELD_PROTOCOL) == Some("secure");

    if let Some(field) = state.field_mut(WEB_FIELD_WS_CERT_FILE) {
        field.visible = is_secure;
    }
    if let Some(field) = state.field_mut(WEB_FIELD_WS_KEY_FILE) {
        field.visible = is_secure;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::popup::PopupState;

    #[test]
    fn test_web_popup_creation() {
        let def = create_web_popup(
            false, true, "9000",
            true, "9001", "secret", "",
            "/path/to/cert", "/path/to/key",
        );
        let state = PopupState::new(def);

        assert_eq!(state.definition.id, PopupId("web"));
        assert_eq!(state.definition.title, "Web Settings");
    }

    #[test]
    fn test_web_popup_tls_visibility() {
        // Non-secure mode - TLS fields hidden
        let def = create_web_popup(
            false, true, "9000",
            true, "9001", "secret", "",
            "", "",
        );
        let state = PopupState::new(def);

        assert!(!state.field(WEB_FIELD_WS_CERT_FILE).unwrap().visible);
        assert!(!state.field(WEB_FIELD_WS_KEY_FILE).unwrap().visible);

        // Secure mode - TLS fields visible
        let def = create_web_popup(
            true, true, "9000",
            true, "9001", "secret", "",
            "", "",
        );
        let state = PopupState::new(def);

        assert!(state.field(WEB_FIELD_WS_CERT_FILE).unwrap().visible);
        assert!(state.field(WEB_FIELD_WS_KEY_FILE).unwrap().visible);
    }
}
