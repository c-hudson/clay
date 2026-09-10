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
pub const WEB_FIELD_REMOTE_LINES: FieldId = FieldId(12);
/// Save-blocking validation error / non-blocking warning line. Sits immediately
/// before Auth Key (the last field) and above the button row. Hidden when there
/// is nothing to say. See `validate_web_settings` below.
pub const WEB_FIELD_VALIDATION_MSG: FieldId = FieldId(13);

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
    remote_initial_lines: i64,
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
            WEB_FIELD_REMOTE_LINES,
            "Remote Lines",
            FieldKind::text(remote_initial_lines.to_string()),
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
        .with_field(Field::new(
            WEB_FIELD_VALIDATION_MSG,
            "",
            FieldKind::error_text(""),
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
        "Remote Lines: Number of scrollback lines sent to web/",
        "  remote clients on initial connect (10-5000, default",
        "  100). Older history is loaded on demand as you scroll",
        "  up.",
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
/// current Port / Custom Cert File selections, and recompute the validation
/// message (see `validate_web_settings`). Returns whether Save should
/// currently be blocked — callers at the Save button/hotkey sites use this to
/// decide whether to close the popup, instead of re-deriving field values
/// themselves.
pub fn update_web_visibility(state: &mut crate::popup::PopupState) -> bool {
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

    apply_validation(&mut state.definition)
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

    apply_validation(def);
}

/// Pull the current field values out of the definition, run
/// `validate_web_settings`, and write the result into the
/// `WEB_FIELD_VALIDATION_MSG` field (text + visibility). Returns
/// `blocks_save` so the two visibility-updater entry points above can hand
/// it straight to their own callers.
fn apply_validation(def: &mut PopupDefinition) -> bool {
    let text_of = |def: &PopupDefinition, id: FieldId| -> String {
        def.get_field(id).and_then(|f| f.kind.get_text()).unwrap_or("").to_string()
    };
    let selected_of = |def: &PopupDefinition, id: FieldId| -> String {
        def.get_field(id).and_then(|f| f.kind.get_selected()).unwrap_or("").to_string()
    };

    let port_mode = selected_of(def, WEB_FIELD_PORT);
    let custom_port = text_of(def, WEB_FIELD_CUSTOM_PORT);
    let password = text_of(def, WEB_FIELD_WS_PASSWORD);
    let custom_cert = selected_of(def, WEB_FIELD_CUSTOM_CERT) == "yes";
    let cert_file = text_of(def, WEB_FIELD_WS_CERT_FILE);
    let key_file = text_of(def, WEB_FIELD_WS_KEY_FILE);
    let allow_list = text_of(def, WEB_FIELD_WS_ALLOW_LIST);
    let remote_lines = text_of(def, WEB_FIELD_REMOTE_LINES);

    let validation = validate_web_settings(
        &port_mode, &custom_port, &password, custom_cert,
        &cert_file, &key_file, &allow_list, &remote_lines,
    );

    if let Some(field) = def.get_field_mut(WEB_FIELD_VALIDATION_MSG) {
        field.visible = validation.message.is_some();
        if let FieldKind::ErrorText { text } = &mut field.kind {
            *text = validation.message.clone().unwrap_or_default();
        }
    }

    validation.blocks_save
}

/// Result of validating the Web Settings form. Same red styling is used for
/// both a blocking error and a non-blocking warning (`blocks_save` is what
/// distinguishes them) — see CLAUDE.md's Critical Rule that a UI change must
/// land in the console TUI, web, and webview-GUI alike: this struct/function
/// is mirrored (message strings included, verbatim) by `validateWebSettings`
/// in `src/web/app.js`, since JS cannot call into Rust.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebValidation {
    /// The message to show, if any. `None` means nothing to show (either the
    /// form is clean, or the web server is disabled).
    pub message: Option<String>,
    /// Whether `message` should prevent Save. `false` for the allow-list
    /// warning, which is intentionally not an error.
    pub blocks_save: bool,
}

impl WebValidation {
    fn ok() -> Self {
        Self { message: None, blocks_save: false }
    }

    fn error(msg: impl Into<String>) -> Self {
        Self { message: Some(msg.into()), blocks_save: true }
    }

    fn warning(msg: impl Into<String>) -> Self {
        Self { message: Some(msg.into()), blocks_save: false }
    }
}

/// Pure validator for the Web Settings form — no `App`, no I/O, so it can be
/// unit-tested directly (see the `tests` module below) and its logic mirrored
/// exactly in JS. All checks apply only when the web server is enabled
/// (`port_mode != "disabled"`); when disabled, nothing is checked and nothing
/// is shown, regardless of what garbage is sitting in the other fields.
///
/// Blocking errors are checked in priority order and the first applicable one
/// wins — only one message is ever shown at a time:
///   1. Empty password — an empty password does not mean "open access"; it
///      means WebSocket password auth is rejected outright (websocket.rs:
///      "Password auth not available. Use an auth key."), so a browser could
///      never log in.
///   2. Port = Custom with a Custom Port value that isn't an integer 1..=65535.
///   3. Custom Cert File = Yes with an empty Cert File or Key File. Presence
///      check only (never stats the filesystem) — `resolve_web_cert_files`
///      (main.rs) requires BOTH to be non-empty or it silently falls back to
///      the auto-generated cert, so a half-filled pair looks configured but
///      isn't.
///   4. Remote Lines that doesn't parse as a number.
///
/// If none of those apply, a non-empty WS Allow List produces a warning
/// (legitimate config — must never block Save).
#[allow(clippy::too_many_arguments)]
pub fn validate_web_settings(
    port_mode: &str,
    custom_port: &str,
    password: &str,
    custom_cert: bool,
    cert_file: &str,
    key_file: &str,
    allow_list: &str,
    remote_lines: &str,
) -> WebValidation {
    if port_mode == "disabled" {
        return WebValidation::ok();
    }

    if password.is_empty() {
        return WebValidation::error("A password is required for web access.");
    }

    if port_mode == "custom" {
        let port_ok = custom_port.trim().parse::<u32>()
            .map(|p| (1..=65535).contains(&p))
            .unwrap_or(false);
        if !port_ok {
            return WebValidation::error("Custom port must be a number from 1 to 65535.");
        }
    }

    // Presence-only, deliberately not stat'd — matches resolve_web_cert_files'
    // own `!is_empty()` check exactly (no trimming there either).
    if custom_cert && (cert_file.is_empty() || key_file.is_empty()) {
        return WebValidation::error("Custom certificate requires both a cert file and a key file.");
    }

    if remote_lines.trim().parse::<i64>().is_err() {
        return WebValidation::error("Remote lines must be a number.");
    }

    if !allow_list.trim().is_empty() {
        return WebValidation::warning("Allow list is set — addresses not listed are silently dropped.");
    }

    WebValidation::ok()
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
            "testkey123", 100,
        );
        let state = PopupState::new(def);

        assert_eq!(state.definition.id, PopupId("web"));
        assert_eq!(state.definition.title, "Web Settings");
        assert_eq!(state.definition.fields.len(), 11);
    }

    #[test]
    fn test_web_popup_port_selection() {
        // Disabled
        let def = create_web_popup(false, 9000, "clay", "", "", "", "", "", 100);
        assert_eq!(
            def.get_field(WEB_FIELD_PORT).and_then(|f| if let FieldKind::Select { options, selected_index } = &f.kind {
                Some(options[*selected_index].value.clone())
            } else { None }),
            Some("disabled".to_string())
        );
        assert!(!def.get_field(WEB_FIELD_CUSTOM_PORT).unwrap().visible);

        // Default port
        let def = create_web_popup(true, 9000, "clay", "", "", "", "", "", 100);
        assert!(!def.get_field(WEB_FIELD_CUSTOM_PORT).unwrap().visible);

        // Custom port
        let def = create_web_popup(true, 1234, "clay", "", "", "", "", "", 100);
        assert!(def.get_field(WEB_FIELD_CUSTOM_PORT).unwrap().visible);
    }

    #[test]
    fn test_web_popup_cert_visibility() {
        // No custom cert configured — fields hidden
        let def = create_web_popup(true, 9000, "clay", "secret", "", "", "", "", 100);
        let state = PopupState::new(def);
        assert!(!state.field(WEB_FIELD_WS_CERT_FILE).unwrap().visible);
        assert!(!state.field(WEB_FIELD_WS_KEY_FILE).unwrap().visible);

        // Custom cert configured — fields visible
        let def = create_web_popup(true, 9000, "clay", "secret", "", "/c", "/k", "", 100);
        let state = PopupState::new(def);
        assert!(state.field(WEB_FIELD_WS_CERT_FILE).unwrap().visible);
        assert!(state.field(WEB_FIELD_WS_KEY_FILE).unwrap().visible);
    }

    #[test]
    fn test_web_popup_remote_lines() {
        let def = create_web_popup(true, 9000, "clay", "", "", "", "", "", 250);
        let state = PopupState::new(def);
        assert_eq!(state.get_text(WEB_FIELD_REMOTE_LINES), Some("250"));
    }

    #[test]
    fn test_web_popup_auth_key_readonly() {
        let def = create_web_popup(true, 9000, "clay", "", "", "", "", "testkey", 100);
        let field = def.get_field(WEB_FIELD_AUTH_KEY).unwrap();
        assert!(!field.is_focusable(), "Auth Key must be read-only (not focusable)");
    }

    #[test]
    fn test_validation_field_not_focusable() {
        // The message line is display-only, never a stop on the tab cycle.
        let def = create_web_popup(true, 9000, "clay", "secret", "", "", "", "testkey", 100);
        let field = def.get_field(WEB_FIELD_VALIDATION_MSG).unwrap();
        assert!(!field.is_focusable());
    }

    #[test]
    fn test_validation_field_hidden_when_clean() {
        let def = create_web_popup(true, 9000, "clay", "secret", "", "", "", "testkey", 100);
        assert!(!def.get_field(WEB_FIELD_VALIDATION_MSG).unwrap().visible);
    }

    #[test]
    fn test_validation_field_visible_when_password_empty() {
        let def = create_web_popup(true, 9000, "clay", "", "", "", "", "testkey", 100);
        let field = def.get_field(WEB_FIELD_VALIDATION_MSG).unwrap();
        assert!(field.visible);
        assert_eq!(field.kind.get_text(), Some("A password is required for web access."));
    }

    #[test]
    fn test_validation_field_hidden_when_disabled_even_with_bad_data() {
        // Port disabled: garbage everywhere else must not surface a message.
        let def = create_web_popup(false, 65535, "clay", "", "notanumber", "", "/only-cert", "", -5);
        assert!(!def.get_field(WEB_FIELD_VALIDATION_MSG).unwrap().visible);
    }

    #[test]
    fn test_update_web_visibility_recomputes_message_and_blocks() {
        let def = create_web_popup(true, 9000, "clay", "secret", "", "", "", "testkey", 100);
        let mut state = PopupState::new(def);
        assert!(!state.field(WEB_FIELD_VALIDATION_MSG).unwrap().visible);

        state.set_text(WEB_FIELD_WS_PASSWORD, String::new());
        let blocks = update_web_visibility(&mut state);
        assert!(blocks);
        assert!(state.field(WEB_FIELD_VALIDATION_MSG).unwrap().visible);
        assert_eq!(
            state.get_text(WEB_FIELD_VALIDATION_MSG),
            Some("A password is required for web access.")
        );

        state.set_text(WEB_FIELD_WS_PASSWORD, "secret".to_string());
        let blocks = update_web_visibility(&mut state);
        assert!(!blocks);
        assert!(!state.field(WEB_FIELD_VALIDATION_MSG).unwrap().visible);
    }

    // ------------------------------------------------------------------
    // validate_web_settings — pure logic
    // ------------------------------------------------------------------

    #[test]
    fn test_validate_disabled_never_blocks_regardless_of_garbage() {
        let v = validate_web_settings("disabled", "not-a-number", "", true, "", "", "*", "nope");
        assert_eq!(v, WebValidation { message: None, blocks_save: false });
    }

    #[test]
    fn test_validate_empty_password_blocks() {
        let v = validate_web_settings("9000", "9000", "", false, "", "", "", "100");
        assert!(v.blocks_save);
        assert_eq!(v.message.as_deref(), Some("A password is required for web access."));
    }

    #[test]
    fn test_validate_custom_port_empty_blocks() {
        let v = validate_web_settings("custom", "", "secret", false, "", "", "", "100");
        assert!(v.blocks_save);
        assert_eq!(v.message.as_deref(), Some("Custom port must be a number from 1 to 65535."));
    }

    #[test]
    fn test_validate_custom_port_non_numeric_blocks() {
        let v = validate_web_settings("custom", "abc", "secret", false, "", "", "", "100");
        assert!(v.blocks_save);
        assert_eq!(v.message.as_deref(), Some("Custom port must be a number from 1 to 65535."));
    }

    #[test]
    fn test_validate_custom_port_zero_blocks() {
        let v = validate_web_settings("custom", "0", "secret", false, "", "", "", "100");
        assert!(v.blocks_save);
        assert_eq!(v.message.as_deref(), Some("Custom port must be a number from 1 to 65535."));
    }

    #[test]
    fn test_validate_custom_port_too_large_blocks() {
        let v = validate_web_settings("custom", "65536", "secret", false, "", "", "", "100");
        assert!(v.blocks_save);
        assert_eq!(v.message.as_deref(), Some("Custom port must be a number from 1 to 65535."));
    }

    #[test]
    fn test_validate_custom_port_valid_passes() {
        let v = validate_web_settings("custom", "65535", "secret", false, "", "", "", "100");
        assert_eq!(v, WebValidation { message: None, blocks_save: false });
        let v = validate_web_settings("custom", "1", "secret", false, "", "", "", "100");
        assert_eq!(v, WebValidation { message: None, blocks_save: false });
    }

    #[test]
    fn test_validate_custom_port_not_checked_when_port_not_custom() {
        // Port = 9000 (fixed): the Custom Port field is hidden and its
        // (irrelevant) contents must never block.
        let v = validate_web_settings("9000", "garbage", "secret", false, "", "", "", "100");
        assert_eq!(v, WebValidation { message: None, blocks_save: false });
    }

    #[test]
    fn test_validate_half_filled_cert_pair_blocks_cert_only() {
        let v = validate_web_settings("9000", "9000", "secret", true, "", "/key.pem", "", "100");
        assert!(v.blocks_save);
        assert_eq!(
            v.message.as_deref(),
            Some("Custom certificate requires both a cert file and a key file.")
        );

        let v = validate_web_settings("9000", "9000", "secret", true, "/cert.pem", "", "", "100");
        assert!(v.blocks_save);
        assert_eq!(
            v.message.as_deref(),
            Some("Custom certificate requires both a cert file and a key file.")
        );
    }

    #[test]
    fn test_validate_both_filled_cert_pair_passes() {
        let v = validate_web_settings("9000", "9000", "secret", true, "/cert.pem", "/key.pem", "", "100");
        assert_eq!(v, WebValidation { message: None, blocks_save: false });
    }

    #[test]
    fn test_validate_custom_cert_no_ignores_empty_paths() {
        // Custom Cert File = No: cert/key fields are hidden and irrelevant.
        let v = validate_web_settings("9000", "9000", "secret", false, "", "", "", "100");
        assert_eq!(v, WebValidation { message: None, blocks_save: false });
    }

    #[test]
    fn test_validate_bad_remote_lines_blocks() {
        let v = validate_web_settings("9000", "9000", "secret", false, "", "", "", "not-a-number");
        assert!(v.blocks_save);
        assert_eq!(v.message.as_deref(), Some("Remote lines must be a number."));

        let v = validate_web_settings("9000", "9000", "secret", false, "", "", "", "");
        assert!(v.blocks_save);
        assert_eq!(v.message.as_deref(), Some("Remote lines must be a number."));
    }

    #[test]
    fn test_validate_nonempty_allow_list_warns_but_does_not_block() {
        let v = validate_web_settings("9000", "9000", "secret", false, "", "", "192.168.1.*", "100");
        assert!(!v.blocks_save);
        assert_eq!(
            v.message.as_deref(),
            Some("Allow list is set — addresses not listed are silently dropped.")
        );
    }

    #[test]
    fn test_validate_clean_form_has_no_message() {
        let v = validate_web_settings("9000", "9000", "secret", false, "", "", "", "100");
        assert_eq!(v, WebValidation { message: None, blocks_save: false });
    }

    #[test]
    fn test_validate_priority_order_password_before_everything() {
        // Empty password + bad custom port + half-filled cert + bad remote
        // lines + non-empty allow list, all at once: password wins.
        let v = validate_web_settings("custom", "-1", "", true, "", "", "*", "nope");
        assert!(v.blocks_save);
        assert_eq!(v.message.as_deref(), Some("A password is required for web access."));
    }

    #[test]
    fn test_validate_priority_order_port_before_cert_and_lines() {
        let v = validate_web_settings("custom", "0", "secret", true, "", "", "*", "nope");
        assert!(v.blocks_save);
        assert_eq!(v.message.as_deref(), Some("Custom port must be a number from 1 to 65535."));
    }

    #[test]
    fn test_validate_priority_order_cert_before_lines_and_allowlist() {
        let v = validate_web_settings("9000", "9000", "secret", true, "", "", "*", "nope");
        assert!(v.blocks_save);
        assert_eq!(
            v.message.as_deref(),
            Some("Custom certificate requires both a cert file and a key file.")
        );
    }

    #[test]
    fn test_validate_priority_order_lines_before_allowlist() {
        let v = validate_web_settings("9000", "9000", "secret", false, "", "", "*", "nope");
        assert!(v.blocks_save);
        assert_eq!(v.message.as_deref(), Some("Remote lines must be a number."));
    }
}
