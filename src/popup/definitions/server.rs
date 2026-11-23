//! The Server popup: a bare `/server` in the console and the SSH console (user request
//! 2026-11-21: the Android app's server settings in every program that starts Clay).
//! This machine's default server and its login, which a bare `clay` attaches to, saved to
//! this machine's client.dat (src/clientconf.rs) - never to settings.dat or a server, so
//! an attached console edits its own machine's, not the one it is attached to.
//!
//! The password and the key are masked, as `/import`'s are: they are the far Clay's
//! login, not something shown again for editing.

use crate::clientconf::ServerPage;
use crate::popup::{Button, ButtonId, Field, FieldId, FieldKind, PopupDefinition, PopupId, PopupLayout, PopupState};

pub const SERVER_FIELD_HOST: FieldId = FieldId(1);
pub const SERVER_FIELD_PORT: FieldId = FieldId(2);
pub const SERVER_FIELD_USERNAME: FieldId = FieldId(3);
pub const SERVER_FIELD_PASSWORD: FieldId = FieldId(4);
pub const SERVER_FIELD_AUTH_KEY: FieldId = FieldId(5);

pub const SERVER_BTN_SAVE: ButtonId = ButtonId(1);
pub const SERVER_BTN_CANCEL: ButtonId = ButtonId(2);

pub const SERVER_POPUP_ID: PopupId = PopupId("server");

/// The popup, filled from client.dat's Server page.
pub fn create_server_popup(page: &ServerPage) -> PopupDefinition {
    PopupDefinition::new(SERVER_POPUP_ID, "Server")
        .with_field(Field::new(SERVER_FIELD_HOST, "Host", FieldKind::text(&page.host)))
        .with_field(Field::new(SERVER_FIELD_PORT, "Port", FieldKind::text(&page.port)))
        .with_field(Field::new(SERVER_FIELD_USERNAME, "Username", FieldKind::text(&page.username)))
        .with_field(Field::new(SERVER_FIELD_PASSWORD, "Password", FieldKind::password(&page.password)))
        .with_field(Field::new(SERVER_FIELD_AUTH_KEY, "Auth Key", FieldKind::password(&page.auth_key)))
        .with_button(Button::new(SERVER_BTN_CANCEL, "Cancel").with_shortcut('C'))
        .with_button(Button::new(SERVER_BTN_SAVE, "Save").primary().with_shortcut('S'))
        .with_layout(PopupLayout {
            label_width: 10,
            min_width: 50,
            max_width_percent: 70,
            center_horizontal: true,
            center_vertical: true,
            modal: true,
            buttons_right_align: true,
            blank_line_before_list: false,
            tab_buttons_only: false,
            anchor_bottom_left: false,
            anchor_x: 0,
        })
        .with_help(help_lines())
}

/// What the fields say now.
pub fn page_of(state: &PopupState) -> ServerPage {
    let text = |id| state.get_text(id).unwrap_or("").to_string();
    ServerPage {
        host: text(SERVER_FIELD_HOST).trim().to_string(),
        port: text(SERVER_FIELD_PORT).trim().to_string(),
        username: text(SERVER_FIELD_USERNAME).trim().to_string(),
        password: text(SERVER_FIELD_PASSWORD),
        auth_key: text(SERVER_FIELD_AUTH_KEY).trim().to_string(),
    }
}

fn help_lines() -> Vec<String> {
    crate::clientconf::SERVER_PAGE_HELP.iter().map(|s| s.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_popup_shows_the_page_and_reads_it_back() {
        let page = ServerPage {
            host: "mud.example".into(),
            port: "4000".into(),
            username: "alice".into(),
            password: "pw".into(),
            auth_key: "key".into(),
        };
        let state = PopupState::new(create_server_popup(&page));
        assert_eq!(state.definition.id, SERVER_POPUP_ID);
        assert_eq!(page_of(&state), page);
    }
}
