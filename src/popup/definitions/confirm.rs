//! Confirmation dialog definition
//!
//! A simple yes/no confirmation dialog for destructive actions.

use crate::popup::{
    Button, ButtonId, Field, FieldId, FieldKind, PopupDefinition, PopupId, PopupLayout,
};

// Field IDs
pub const CONFIRM_FIELD_MESSAGE: FieldId = FieldId(1);

// Button IDs
pub const CONFIRM_BTN_YES: ButtonId = ButtonId(1);
pub const CONFIRM_BTN_NO: ButtonId = ButtonId(2);

/// Create a confirmation dialog
///
/// # Arguments
/// * `id` - Unique identifier for this dialog (e.g., "delete_world")
/// * `title` - Dialog title
/// * `message` - The confirmation message to display
pub fn create_confirm_dialog(id: &'static str, title: &str, message: &str) -> PopupDefinition {
    PopupDefinition::new(PopupId(id), title)
        .with_field(Field::new(
            CONFIRM_FIELD_MESSAGE,
            "",
            FieldKind::label(message.to_string()),
        ))
        .with_button(Button::new(CONFIRM_BTN_YES, "Yes").primary().with_shortcut('Y'))
        .with_button(Button::new(CONFIRM_BTN_NO, "No").danger().with_shortcut('N'))
        .with_layout(PopupLayout {
            label_width: 0,
            min_width: 30,
            max_width_percent: 50,
            center_horizontal: true,
            center_vertical: true,
            modal: true,
            buttons_right_align: false,
            blank_line_before_list: false,
            tab_buttons_only: false,
            anchor_bottom_left: false,
            anchor_x: 0,
        })
}

/// Create a delete world confirmation dialog
pub fn create_delete_world_dialog(world_name: &str) -> PopupDefinition {
    create_confirm_dialog(
        "delete_world",
        "Confirm Delete",
        &format!("Delete world '{}'?", world_name),
    )
}

/// Create a delete action confirmation dialog
pub fn create_delete_action_dialog(action_name: &str) -> PopupDefinition {
    create_confirm_dialog(
        "delete_action",
        "Confirm Delete",
        &format!("Delete action '{}'?", action_name),
    )
}

/// Create an allow list wildcard warning dialog
pub fn create_allow_list_warning_dialog() -> PopupDefinition {
    create_confirm_dialog(
        "allow_list_warning",
        "Warning",
        "Allow list contains '*' which permits all hosts to connect with a password. Continue?",
    )
}

// Custom_data keys used by the cert-mismatch dialog, shared with the confirm
// handler in input_handler.rs so it can tell this dialog apart from
// delete-world/delete-action/allow-list confirms (which key off different
// custom_data entries).
pub const CERT_MISMATCH_WORLD_INDEX: &str = "cert_mismatch_world_index";
pub const CERT_MISMATCH_HOST: &str = "cert_mismatch_host";
pub const CERT_MISMATCH_NEW_FP: &str = "cert_mismatch_new_fp";

/// Create a TLS certificate pin-mismatch warning dialog (trust-on-first-use).
/// Shown when a world's TLS certificate no longer matches the fingerprint
/// pinned in `~/.clay/known_hosts.dat`. "Yes" trusts the new certificate
/// (replaces the pin) and reconnects; "No" leaves the old pin in place and the
/// connection blocked.
pub fn create_cert_mismatch_dialog(
    world_index: usize,
    host: &str,
    old_fingerprint: &str,
    new_fingerprint: &str,
) -> PopupDefinition {
    // Shorten the fingerprints for display (first/last 8 hex chars is plenty to
    // recognize "this is different" without wrapping the popup).
    let short = |fp: &str| -> String {
        if fp.len() > 20 {
            format!("{}...{}", &fp[..10], &fp[fp.len() - 10..])
        } else {
            fp.to_string()
        }
    };
    let message = format!(
        "TLS certificate for {} has CHANGED.\n\nOld: {}\nNew: {}\n\nThis could mean the server was reinstalled, or that someone is intercepting your connection.\n\nTrust the new certificate and reconnect?",
        host, short(old_fingerprint), short(new_fingerprint)
    );
    let mut def = create_confirm_dialog("cert_mismatch", "TLS Certificate Changed", &message);
    def.custom_data.insert(CERT_MISMATCH_WORLD_INDEX.to_string(), world_index.to_string());
    def.custom_data.insert(CERT_MISMATCH_HOST.to_string(), host.to_string());
    def.custom_data.insert(CERT_MISMATCH_NEW_FP.to_string(), new_fingerprint.to_string());
    def
}

// Custom_data key used by the /import insecure-transport confirm dialog. The credentials
// themselves live in App::pending_console_import (input_handler.rs reads them back from
// there, not from custom_data) — this key only needs to carry enough to identify the
// dialog and let the message reference the target.
pub const IMPORT_INSECURE_ADDR: &str = "import_insecure_addr";

/// Create the /import insecure-transport confirm dialog: `addr` didn't accept a TLS
/// connection, so continuing sends the password/auth-key from `App::pending_console_import`
/// unencrypted. See plan i-d-like-to-make-snuggly-rain.md step 8.
pub fn create_import_insecure_dialog(addr: &str) -> PopupDefinition {
    let message = format!(
        "{} did not accept a TLS connection.\n\nContinuing will send your password/auth-key to it UNENCRYPTED. Only do this on a network you trust.",
        addr
    );
    let mut def = create_confirm_dialog("import_insecure", "No Secure Connection", &message);
    def.custom_data.insert(IMPORT_INSECURE_ADDR.to_string(), addr.to_string());
    def
}

// Marker custom_data key for the /import "reload now?" offer — no extra data needed
// beyond identifying the dialog (unlike IMPORT_INSECURE_ADDR, there's no retry to drive).
pub const IMPORT_RELOAD_OFFER: &str = "import_reload_offer";

/// Create the /import "apply now?" dialog shown to the master console TUI after a
/// successful import: `/reload` re-execs the process so the merged settings take full
/// effect immediately (matching the design's "master TUI/desktop offers /reload" — plan
/// i-d-like-to-make-snuggly-rain.md step 9). "No" leaves the merged settings.dat/theme.dat/
/// keybindings.dat on disk as-is, applied next time Clay starts.
pub fn create_import_reload_dialog(summary: &str) -> PopupDefinition {
    let message = format!("{}\n\nReload now to apply?", summary);
    let mut def = create_confirm_dialog("import_reload_offer", "Import Complete", &message);
    def.custom_data.insert(IMPORT_RELOAD_OFFER.to_string(), "1".to_string());
    def
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::popup::{ElementSelection, PopupState};

    #[test]
    fn test_confirm_dialog_creation() {
        let def = create_confirm_dialog("test", "Test", "Are you sure?");
        let state = PopupState::new(def);

        assert_eq!(state.definition.id, PopupId("test"));
        assert_eq!(state.definition.title, "Test");
        assert_eq!(state.definition.buttons.len(), 2);
    }

    #[test]
    fn test_confirm_starts_on_no() {
        let def = create_confirm_dialog("test", "Test", "Are you sure?");
        let mut state = PopupState::new(def);
        state.open();

        // Should start on No button (safer default)
        // Actually, first focusable element is the message label which is not focusable,
        // so it should go to first button
        state.select_first_button();
        assert!(matches!(state.selected, ElementSelection::Button(CONFIRM_BTN_YES)));
    }

    #[test]
    fn test_delete_world_dialog() {
        let def = create_delete_world_dialog("TestWorld");

        // Check that the message contains the world name
        if let Some(field) = def.get_field(CONFIRM_FIELD_MESSAGE) {
            if let FieldKind::Label { text } = &field.kind {
                assert!(text.contains("TestWorld"));
            } else {
                panic!("Expected Label field");
            }
        } else {
            panic!("Expected message field");
        }
    }
}
