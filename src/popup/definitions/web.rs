//! Web settings popup definition
//!
//! Allows editing the web/WebSocket server settings. The server is always
//! TLS-capable for remote clients now (trust-on-first-use auto cert, or a
//! user-provided one); localhost is always served plain. See CLAUDE.md
//! "Connection Security" and `resolve_web_cert_files` in main.rs.

use crate::popup::{
    Button, ButtonId, Field, FieldId, FieldKind, PopupDefinition, PopupId, PopupLayout,
    SelectOption,
};

// Field IDs
pub const WEB_FIELD_PORT: FieldId = FieldId(3);
pub const WEB_FIELD_CUSTOM_PORT: FieldId = FieldId(4);
pub const WEB_FIELD_CUSTOM_CERT: FieldId = FieldId(5);
pub const WEB_FIELD_WS_PASSWORD: FieldId = FieldId(6);
pub const WEB_FIELD_WS_ALLOW_LIST: FieldId = FieldId(7);
pub const WEB_FIELD_WS_CERT_FILE: FieldId = FieldId(8);
pub const WEB_FIELD_WS_KEY_FILE: FieldId = FieldId(9);
pub const WEB_FIELD_AUTH_KEY: FieldId = FieldId(10);
pub const WEB_FIELD_WEB_PATH: FieldId = FieldId(11);

// Button IDs
pub const WEB_BTN_SAVE: ButtonId = ButtonId(1);
pub const WEB_BTN_CANCEL: ButtonId = ButtonId(2);
pub const WEB_BTN_MODIFY_KEY: ButtonId = ButtonId(3);

/// Port field options: disabled, the default 9000, or a user-defined port
/// (revealed as a separate "Custom Port" text field).
pub fn port_options() -> Vec<SelectOption> {
    vec![
        SelectOption::new("disabled", "Disabled"),
        SelectOption::new("9000", "9000"),
        SelectOption::new("custom", "Custom"),
    ]
}

/// Custom Cert File options: No (use the auto-generated cert) or Yes (reveals
/// the cert/key file path fields).
pub fn custom_cert_options() -> Vec<SelectOption> {
    vec![
        SelectOption::new("no", "No"),
        SelectOption::new("yes", "Yes"),
    ]
}

/// Create the web settings popup definition with current values.
#[allow(clippy::too_many_arguments)]
pub fn create_web_popup(
    http_enabled: bool,
    http_port: u16,
    web_path: &str,
    ws_password: &str,
    ws_allow_list: &str,
    ws_cert_file: &str,
    ws_key_file: &str,
    auth_key: &str,
) -> PopupDefinition {
    let port_selected = if !http_enabled {
        "disabled"
    } else if http_port == 9000 {
        "9000"
    } else {
        "custom"
    };
    let port_idx = port_options().iter().position(|o| o.value == port_selected).unwrap_or(0);
    let has_custom_cert = !ws_cert_file.is_empty() && !ws_key_file.is_empty();
    let cert_idx = if has_custom_cert { 1 } else { 0 };

    let mut def = PopupDefinition::new(PopupId("web"), "Web Settings")
        .with_field(Field::new(
            WEB_FIELD_PORT,
            "Port",
            FieldKind::select(port_options(), port_idx),
        ))
        .with_field(Field::new(
            WEB_FIELD_CUSTOM_PORT,
            "Custom Port",
            FieldKind::text(http_port.to_string()),
        ))
        .with_field(Field::new(
            WEB_FIELD_WEB_PATH,
            "Web Path",
            FieldKind::text(web_path),
        ))
        .with_field(Field::new(
            WEB_FIELD_WS_PASSWORD,
            "Password",
            FieldKind::text(ws_password),
        ))
        .with_field(Field::new(
            WEB_FIELD_WS_ALLOW_LIST,
            "WS Allow List",
            FieldKind::text(ws_allow_list),
        ))
        .with_field(Field::new(
            WEB_FIELD_CUSTOM_CERT,
            "Custom Cert File",
            FieldKind::select(custom_cert_options(), cert_idx),
        ))
        .with_field(Field::new(
            WEB_FIELD_WS_CERT_FILE,
            "Cert File",
            FieldKind::text(ws_cert_file),
        ))
        .with_field(Field::new(
            WEB_FIELD_WS_KEY_FILE,
            "Key File",
            FieldKind::text(ws_key_file),
        ))
        .with_field(
            Field::new(
                WEB_FIELD_AUTH_KEY,
                "Auth Key",
                FieldKind::text(auth_key),
            )
            .disabled(),
        )
        .with_button(Button::new(WEB_BTN_MODIFY_KEY, "Modify Key").with_shortcut('M'))
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
            tab_buttons_only: false,
            anchor_bottom_left: false,
            anchor_x: 0,
        });

    def = def.with_help(web_help_text());

    update_web_visibility_def(&mut def);

    def
}

/// Help text for the Web Settings popup
fn web_help_text() -> Vec<String> {
    vec![
        "Web Settings - Remote Access",
        "",
        "These settings let you access Clay from a web",
        "browser or mobile device on your network. The",
        "server is always TLS-encrypted for remote clients",
        "(auto-generated certificate, or your own — see",
        "Custom Cert File below); connections from this",
        "same machine (localhost) are always unencrypted,",
        "so the desktop app never shows a certificate prompt.",
        "",
        "Other Clay instances connecting to this one (remote",
        "console, another Clay's WebView, the Android app)",
        "trust the certificate automatically the first time",
        "and only ask for confirmation if it later changes —",
        "so there is nothing to configure for that to work.",
        "A plain web browser will show a one-time \"not",
        "secure\" warning for the self-signed certificate;",
        "use Custom Cert File to supply a CA-signed one and",
        "avoid that.",
        "",
        "Port: Disabled turns the web server off. 9000 is",
        "  the default port. Custom lets you pick your own",
        "  (shown in the Custom Port field below).",
        "",
        "Web Path: Stealth path prefix for the web UI (default",
        "  \"clay\" — UI served only at /clay/, everything else",
        "  is silently dropped for non-localhost connections).",
        "  Leave empty to restore legacy mode (UI at \"/\", old",
        "  bookmarks and old Android APKs keep working, but",
        "  scanners can see the login page). Localhost (the GUI",
        "  WebView) always works at both paths, no config",
        "  needed. Android app with an auth key can knock to",
        "  connect from anywhere even in stealth mode; without",
        "  a knock, non-localhost devices need to be on the WS",
        "  Allow List.",
        "",
        "Password: Required for WebSocket clients (web,",
        "  mobile, remote console) to connect. Accepted",
        "  from any address when the Allow List is empty; when",
        "  an Allow List is set, only from listed addresses.",
        "  A wrong password bans the address after 5 failed",
        "  attempts (localhost excepted) — this applies even",
        "  to addresses on the Allow List.",
        "",
        "WS Allow List: Comma-separated IPs, IP wildcards,",
        "  or hostnames allowed to connect. Empty = allow all",
        "  (password or Auth Key still required).",
        "  Examples: 192.168.1.*, *.rd.shawcable.net",
        "  When set, addresses NOT on the list are dropped at",
        "  the TCP level: no page, no TLS handshake, no reply.",
        "  Their only way in is the Android auth-key knock,",
        "  which grants WebSocket access only (never the web",
        "  UI). Multiuser mode has no Auth Key, so unlisted",
        "  addresses cannot connect there at all. Addresses ON",
        "  the Allow List are never banned for bad paths, http/",
        "  https typos, or other connection probes — only a",
        "  bare \"*\" entry does NOT get this protection (it",
        "  means \"let everyone in\", not \"never ban anyone\").",
        "",
        "Custom Cert File: No (default) uses an automatically",
        "  generated, self-signed certificate — nothing to",
        "  configure. Yes lets you supply your own cert/key",
        "  PEM files (e.g. a CA-signed certificate) instead.",
        "",
        "Auth Key: Device authentication key for passwordless",
        "  login from the Android app or trusted devices.",
        "  Read-only here — use Modify Key to copy, regenerate,",
        "  or delete it. The Android app also uses it to knock:",
        "  it proves the key before any web request, which is",
        "  the only way in from an address not on the Allow",
        "  List. Regen or delete takes effect immediately.",
    ].into_iter().map(|s| s.to_string()).collect()
}

/// Update visibility of the Custom Port and cert/key fields based on the
/// current Port / Custom Cert File selections.
pub fn update_web_visibility(state: &mut crate::popup::PopupState) {
    let show_custom_port = state.get_selected(WEB_FIELD_PORT) == Some("custom");
    if let Some(field) = state.field_mut(WEB_FIELD_CUSTOM_PORT) {
        field.visible = show_custom_port;
    }

    let show_cert_fields = state.get_selected(WEB_FIELD_CUSTOM_CERT) == Some("yes");
    if let Some(field) = state.field_mut(WEB_FIELD_WS_CERT_FILE) {
        field.visible = show_cert_fields;
    }
    if let Some(field) = state.field_mut(WEB_FIELD_WS_KEY_FILE) {
        field.visible = show_cert_fields;
    }
}

/// Same as `update_web_visibility` but operates directly on a `PopupDefinition`
/// (used at creation time, before a `PopupState` wraps it).
fn update_web_visibility_def(def: &mut PopupDefinition) {
    let show_custom_port = def.get_field(WEB_FIELD_PORT)
        .and_then(|f| if let FieldKind::Select { options, selected_index } = &f.kind {
            options.get(*selected_index).map(|o| o.value == "custom")
        } else { None })
        .unwrap_or(false);
    if let Some(field) = def.get_field_mut(WEB_FIELD_CUSTOM_PORT) {
        field.visible = show_custom_port;
    }

    let show_cert_fields = def.get_field(WEB_FIELD_CUSTOM_CERT)
        .and_then(|f| if let FieldKind::Select { options, selected_index } = &f.kind {
            options.get(*selected_index).map(|o| o.value == "yes")
        } else { None })
        .unwrap_or(false);
    if let Some(field) = def.get_field_mut(WEB_FIELD_WS_CERT_FILE) {
        field.visible = show_cert_fields;
    }
    if let Some(field) = def.get_field_mut(WEB_FIELD_WS_KEY_FILE) {
        field.visible = show_cert_fields;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::popup::PopupState;

    #[test]
    fn test_web_popup_creation() {
        let def = create_web_popup(
            true, 9000, "clay",
            "secret", "",
            "/path/to/cert", "/path/to/key",
            "testkey123",
        );
        let state = PopupState::new(def);

        assert_eq!(state.definition.id, PopupId("web"));
        assert_eq!(state.definition.title, "Web Settings");
    }

    #[test]
    fn test_web_popup_port_selection() {
        // Disabled
        let def = create_web_popup(false, 9000, "clay", "", "", "", "", "");
        assert_eq!(
            def.get_field(WEB_FIELD_PORT).and_then(|f| if let FieldKind::Select { options, selected_index } = &f.kind {
                Some(options[*selected_index].value.clone())
            } else { None }),
            Some("disabled".to_string())
        );
        assert!(!def.get_field(WEB_FIELD_CUSTOM_PORT).unwrap().visible);

        // Default port
        let def = create_web_popup(true, 9000, "clay", "", "", "", "", "");
        assert!(!def.get_field(WEB_FIELD_CUSTOM_PORT).unwrap().visible);

        // Custom port
        let def = create_web_popup(true, 1234, "clay", "", "", "", "", "");
        assert!(def.get_field(WEB_FIELD_CUSTOM_PORT).unwrap().visible);
    }

    #[test]
    fn test_web_popup_cert_visibility() {
        // No custom cert configured — fields hidden
        let def = create_web_popup(true, 9000, "clay", "secret", "", "", "", "");
        let state = PopupState::new(def);
        assert!(!state.field(WEB_FIELD_WS_CERT_FILE).unwrap().visible);
        assert!(!state.field(WEB_FIELD_WS_KEY_FILE).unwrap().visible);

        // Custom cert configured — fields visible
        let def = create_web_popup(true, 9000, "clay", "secret", "", "/c", "/k", "");
        let state = PopupState::new(def);
        assert!(state.field(WEB_FIELD_WS_CERT_FILE).unwrap().visible);
        assert!(state.field(WEB_FIELD_WS_KEY_FILE).unwrap().visible);
    }

    #[test]
    fn test_web_popup_auth_key_readonly() {
        let def = create_web_popup(true, 9000, "clay", "", "", "", "", "testkey");
        let field = def.get_field(WEB_FIELD_AUTH_KEY).unwrap();
        assert!(!field.is_focusable(), "Auth Key must be read-only (not focusable)");
    }
}
