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
pub const WEB_FIELD_AUTH_KEY: FieldId = FieldId(10);
pub const WEB_FIELD_WEB_PATH: FieldId = FieldId(11);

// Button IDs
pub const WEB_BTN_SAVE: ButtonId = ButtonId(1);
pub const WEB_BTN_CANCEL: ButtonId = ButtonId(2);
pub const WEB_BTN_REGEN_KEY: ButtonId = ButtonId(3);
pub const WEB_BTN_COPY_KEY: ButtonId = ButtonId(4);

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
    web_path: &str,
    ws_password: &str,
    ws_allow_list: &str,
    ws_cert_file: &str,
    ws_key_file: &str,
    auth_key: &str,
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
            WEB_FIELD_WEB_PATH,
            "Web Path",
            FieldKind::text(web_path),
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
        .with_field(Field::new(
            WEB_FIELD_AUTH_KEY,
            "Auth Key",
            FieldKind::text(auth_key),
        ))
        .with_button(Button::new(WEB_BTN_COPY_KEY, "Copy Key").with_shortcut('K'))
        .with_button(Button::new(WEB_BTN_REGEN_KEY, "Regen Key").with_shortcut('R'))
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

/// Help text for the Web Settings popup
fn web_help_text() -> Vec<String> {
    vec![
        "Web Settings - Remote Access",
        "",
        "These settings let you access Clay from a web",
        "browser or mobile device on your network.",
        "",
        "Protocol: Choose Secure (HTTPS/WSS) or Non-Secure",
        "  (HTTP/WS). Secure requires TLS certificate files.",
        "",
        "HTTP Enabled: Starts a web server so you can open",
        "  Clay in a browser at http://yourhost:port.",
        "",
        "HTTP Port: The port number for the web server.",
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
        "WS Password: Password required for WebSocket clients",
        "  (web, mobile, remote console) to connect. Accepted",
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
        "TLS Cert File: Path to your TLS/SSL certificate",
        "  file (.pem or .crt) for secure connections.",
        "",
        "TLS Key File: Path to your TLS/SSL private key",
        "  file (.pem or .key) for secure connections.",
        "",
        "Auth Key: Device authentication key for passwordless",
        "  login from the Android app or trusted devices.",
        "  Use Regen Key to generate a new key. Copy the key",
        "  to paste into the Android app's settings.",
        "  The Android app also uses it to knock: it proves",
        "  the key before any web request, which is the only",
        "  way in from an address not on the Allow List.",
        "  Regen or revoke takes effect immediately.",
    ].into_iter().map(|s| s.to_string()).collect()
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
            false, true, "9000", "clay",
            "secret", "",
            "/path/to/cert", "/path/to/key",
            "testkey123",
        );
        let state = PopupState::new(def);

        assert_eq!(state.definition.id, PopupId("web"));
        assert_eq!(state.definition.title, "Web Settings");
    }

    #[test]
    fn test_web_popup_tls_visibility() {
        // Non-secure mode - TLS fields hidden
        let def = create_web_popup(
            false, true, "9000", "clay",
            "secret", "",
            "", "",
            "",
        );
        let state = PopupState::new(def);

        assert!(!state.field(WEB_FIELD_WS_CERT_FILE).unwrap().visible);
        assert!(!state.field(WEB_FIELD_WS_KEY_FILE).unwrap().visible);

        // Secure mode - TLS fields visible
        let def = create_web_popup(
            true, true, "9000", "clay",
            "secret", "",
            "", "",
            "",
        );
        let state = PopupState::new(def);

        assert!(state.field(WEB_FIELD_WS_CERT_FILE).unwrap().visible);
        assert!(state.field(WEB_FIELD_WS_KEY_FILE).unwrap().visible);
    }
}
