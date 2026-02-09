//! World editor popup definition
//!
//! Allows editing world-specific settings (MUD, Slack, Discord).

use crate::popup::{
    Button, ButtonId, Field, FieldId, FieldKind, PopupDefinition, PopupId, PopupLayout,
    SelectOption,
};

// Field IDs - common
pub const WORLD_FIELD_NAME: FieldId = FieldId(1);
pub const WORLD_FIELD_TYPE: FieldId = FieldId(2);
// Field IDs - MUD
pub const WORLD_FIELD_HOSTNAME: FieldId = FieldId(10);
pub const WORLD_FIELD_PORT: FieldId = FieldId(11);
pub const WORLD_FIELD_USER: FieldId = FieldId(12);
pub const WORLD_FIELD_PASSWORD: FieldId = FieldId(13);
pub const WORLD_FIELD_USE_SSL: FieldId = FieldId(14);
pub const WORLD_FIELD_LOG_ENABLED: FieldId = FieldId(15);
pub const WORLD_FIELD_ENCODING: FieldId = FieldId(16);
pub const WORLD_FIELD_AUTO_CONNECT: FieldId = FieldId(17);
pub const WORLD_FIELD_KEEP_ALIVE: FieldId = FieldId(18);
pub const WORLD_FIELD_KEEP_ALIVE_CMD: FieldId = FieldId(19);
pub const WORLD_FIELD_GMCP_PACKAGES: FieldId = FieldId(20);
// Field IDs - Slack
pub const WORLD_FIELD_SLACK_TOKEN: FieldId = FieldId(30);
pub const WORLD_FIELD_SLACK_CHANNEL: FieldId = FieldId(31);
pub const WORLD_FIELD_SLACK_WORKSPACE: FieldId = FieldId(32);
// Field IDs - Discord
pub const WORLD_FIELD_DISCORD_TOKEN: FieldId = FieldId(40);
pub const WORLD_FIELD_DISCORD_GUILD: FieldId = FieldId(41);
pub const WORLD_FIELD_DISCORD_CHANNEL: FieldId = FieldId(42);
pub const WORLD_FIELD_DISCORD_DM_USER: FieldId = FieldId(43);

// Button IDs
pub const WORLD_BTN_SAVE: ButtonId = ButtonId(1);
pub const WORLD_BTN_CANCEL: ButtonId = ButtonId(2);
pub const WORLD_BTN_DELETE: ButtonId = ButtonId(3);
pub const WORLD_BTN_CONNECT: ButtonId = ButtonId(4);

/// World type for the popup
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WorldType {
    Mud,
    Slack,
    Discord,
}

impl WorldType {
    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "slack" => WorldType::Slack,
            "discord" => WorldType::Discord,
            _ => WorldType::Mud,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            WorldType::Mud => "mud",
            WorldType::Slack => "slack",
            WorldType::Discord => "discord",
        }
    }
}

/// World type options
pub fn world_type_options() -> Vec<SelectOption> {
    vec![
        SelectOption::new("mud", "MUD"),
        SelectOption::new("slack", "Slack"),
        SelectOption::new("discord", "Discord"),
    ]
}

/// Encoding options
pub fn encoding_options() -> Vec<SelectOption> {
    vec![
        SelectOption::new("utf8", "UTF-8"),
        SelectOption::new("latin1", "Latin-1"),
        SelectOption::new("fansi", "FANSI"),
    ]
}

/// Auto-connect options
pub fn auto_connect_options() -> Vec<SelectOption> {
    vec![
        SelectOption::new("connect", "Connect"),
        SelectOption::new("prompt", "Prompt"),
        SelectOption::new("moo_prompt", "MOO Prompt"),
        SelectOption::new("none", "None"),
    ]
}

/// Keep-alive options
pub fn keep_alive_options() -> Vec<SelectOption> {
    vec![
        SelectOption::new("nop", "NOP"),
        SelectOption::new("custom", "Custom"),
        SelectOption::new("generic", "Generic"),
    ]
}

/// World settings for the popup
#[derive(Debug, Clone, Default)]
pub struct WorldSettings {
    // Common
    pub name: String,
    pub world_type: String,
    // MUD
    pub hostname: String,
    pub port: String,
    pub user: String,
    pub password: String,
    pub use_ssl: bool,
    pub log_enabled: bool,
    pub encoding: String,
    pub auto_connect: String,
    pub keep_alive: String,
    pub keep_alive_cmd: String,
    pub gmcp_packages: String,
    // Slack
    pub slack_token: String,
    pub slack_channel: String,
    pub slack_workspace: String,
    // Discord
    pub discord_token: String,
    pub discord_guild: String,
    pub discord_channel: String,
    pub discord_dm_user: String,
}

/// Create the world editor popup definition with current values
pub fn create_world_editor_popup(settings: &WorldSettings) -> PopupDefinition {
    let world_type = WorldType::parse(&settings.world_type);
    let world_type_idx = match world_type {
        WorldType::Mud => 0,
        WorldType::Slack => 1,
        WorldType::Discord => 2,
    };

    let encoding_idx = match settings.encoding.as_str() {
        "latin1" => 1,
        "fansi" => 2,
        _ => 0,
    };

    let auto_connect_idx = match settings.auto_connect.as_str() {
        "prompt" => 1,
        "moo_prompt" => 2,
        "none" => 3,
        _ => 0,
    };

    let keep_alive_idx = match settings.keep_alive.as_str() {
        "custom" => 1,
        "generic" => 2,
        _ => 0,
    };

    let show_keep_alive_cmd = settings.keep_alive == "custom";

    let mut def = PopupDefinition::new(PopupId("world_editor"), "World Settings")
        // Common fields
        .with_field(Field::new(
            WORLD_FIELD_NAME,
            "World",
            FieldKind::text(&settings.name),
        ))
        .with_field(Field::new(
            WORLD_FIELD_TYPE,
            "Type",
            FieldKind::select(world_type_options(), world_type_idx),
        ))
        // MUD fields
        .with_field(Field::new(
            WORLD_FIELD_HOSTNAME,
            "Hostname",
            FieldKind::text(&settings.hostname),
        ))
        .with_field(Field::new(
            WORLD_FIELD_PORT,
            "Port",
            FieldKind::text(&settings.port),
        ))
        .with_field(Field::new(
            WORLD_FIELD_USER,
            "User",
            FieldKind::text(&settings.user),
        ))
        .with_field(Field::new(
            WORLD_FIELD_PASSWORD,
            "Password",
            FieldKind::text(&settings.password),
        ))
        .with_field(Field::new(
            WORLD_FIELD_USE_SSL,
            "Use SSL",
            FieldKind::toggle(settings.use_ssl),
        ))
        .with_field(Field::new(
            WORLD_FIELD_LOG_ENABLED,
            "Log File",
            FieldKind::toggle(settings.log_enabled),
        ))
        .with_field(Field::new(
            WORLD_FIELD_ENCODING,
            "Encoding",
            FieldKind::select(encoding_options(), encoding_idx),
        ))
        .with_field(Field::new(
            WORLD_FIELD_AUTO_CONNECT,
            "Auto Login",
            FieldKind::select(auto_connect_options(), auto_connect_idx),
        ))
        .with_field(Field::new(
            WORLD_FIELD_KEEP_ALIVE,
            "Keep Alive",
            FieldKind::select(keep_alive_options(), keep_alive_idx),
        ))
        .with_field(Field::new(
            WORLD_FIELD_KEEP_ALIVE_CMD,
            "KA Command",
            FieldKind::text(&settings.keep_alive_cmd),
        ))
        .with_field(Field::new(
            WORLD_FIELD_GMCP_PACKAGES,
            "GMCP Packages",
            FieldKind::text(&settings.gmcp_packages),
        ))
        // Slack fields
        .with_field(Field::new(
            WORLD_FIELD_SLACK_TOKEN,
            "Token",
            FieldKind::password(&settings.slack_token),
        ))
        .with_field(Field::new(
            WORLD_FIELD_SLACK_CHANNEL,
            "Channel",
            FieldKind::text(&settings.slack_channel),
        ))
        .with_field(Field::new(
            WORLD_FIELD_SLACK_WORKSPACE,
            "Workspace",
            FieldKind::text(&settings.slack_workspace),
        ))
        // Discord fields
        .with_field(Field::new(
            WORLD_FIELD_DISCORD_TOKEN,
            "Token",
            FieldKind::password(&settings.discord_token),
        ))
        .with_field(Field::new(
            WORLD_FIELD_DISCORD_GUILD,
            "Guild",
            FieldKind::text(&settings.discord_guild),
        ))
        .with_field(Field::new(
            WORLD_FIELD_DISCORD_CHANNEL,
            "Channel",
            FieldKind::text(&settings.discord_channel),
        ))
        .with_field(Field::new(
            WORLD_FIELD_DISCORD_DM_USER,
            "DM User",
            FieldKind::text(&settings.discord_dm_user),
        ))
        // Buttons - Tab cycles: Save -> Cancel -> Connect -> Delete -> Save
        .with_button(Button::new(WORLD_BTN_SAVE, "Save").primary().with_shortcut('S').with_tab_index(0))
        .with_button(Button::new(WORLD_BTN_CANCEL, "Cancel").with_shortcut('C').with_tab_index(1))
        .with_button(Button::new(WORLD_BTN_DELETE, "Delete").danger().with_shortcut('D').left_align().with_tab_index(3))
        .with_button(Button::new(WORLD_BTN_CONNECT, "Connect").with_shortcut('O').with_tab_index(2))
        .with_layout(PopupLayout {
            label_width: 12,
            min_width: 50,
            max_width_percent: 70,
            center_horizontal: true,
            center_vertical: true,
            modal: true,
            buttons_right_align: false,
            blank_line_before_list: false,
            tab_buttons_only: true,
        });

    // Set field visibility based on world type
    update_field_visibility(&mut def, world_type, show_keep_alive_cmd);

    def
}

/// Update field visibility based on world type
pub fn update_field_visibility(def: &mut PopupDefinition, world_type: WorldType, show_keep_alive_cmd: bool) {
    // MUD fields
    let mud_fields = [
        WORLD_FIELD_HOSTNAME, WORLD_FIELD_PORT, WORLD_FIELD_USER, WORLD_FIELD_PASSWORD,
        WORLD_FIELD_USE_SSL, WORLD_FIELD_LOG_ENABLED, WORLD_FIELD_ENCODING,
        WORLD_FIELD_AUTO_CONNECT, WORLD_FIELD_KEEP_ALIVE, WORLD_FIELD_GMCP_PACKAGES,
    ];

    // Slack fields
    let slack_fields = [
        WORLD_FIELD_SLACK_TOKEN, WORLD_FIELD_SLACK_CHANNEL, WORLD_FIELD_SLACK_WORKSPACE,
    ];

    // Discord fields
    let discord_fields = [
        WORLD_FIELD_DISCORD_TOKEN, WORLD_FIELD_DISCORD_GUILD,
        WORLD_FIELD_DISCORD_CHANNEL, WORLD_FIELD_DISCORD_DM_USER,
    ];

    // Show/hide based on world type
    for id in mud_fields {
        if let Some(field) = def.get_field_mut(id) {
            field.visible = world_type == WorldType::Mud;
        }
    }

    // Keep-alive command only visible for MUD with custom keep-alive
    if let Some(field) = def.get_field_mut(WORLD_FIELD_KEEP_ALIVE_CMD) {
        field.visible = world_type == WorldType::Mud && show_keep_alive_cmd;
    }

    // Log file visible for all types
    if let Some(field) = def.get_field_mut(WORLD_FIELD_LOG_ENABLED) {
        field.visible = true;
    }

    for id in slack_fields {
        if let Some(field) = def.get_field_mut(id) {
            field.visible = world_type == WorldType::Slack;
        }
    }

    for id in discord_fields {
        if let Some(field) = def.get_field_mut(id) {
            field.visible = world_type == WorldType::Discord;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::popup::PopupState;

    #[test]
    fn test_world_editor_mud() {
        let settings = WorldSettings {
            name: "TestMUD".to_string(),
            world_type: "mud".to_string(),
            hostname: "mud.example.com".to_string(),
            port: "4000".to_string(),
            ..Default::default()
        };
        let def = create_world_editor_popup(&settings);
        let state = PopupState::new(def);

        assert_eq!(state.definition.id, PopupId("world_editor"));
        assert_eq!(state.get_text(WORLD_FIELD_NAME), Some("TestMUD"));
        assert_eq!(state.get_text(WORLD_FIELD_HOSTNAME), Some("mud.example.com"));

        // MUD fields should be visible
        assert!(state.field(WORLD_FIELD_HOSTNAME).unwrap().visible);
        // Slack fields should be hidden
        assert!(!state.field(WORLD_FIELD_SLACK_TOKEN).unwrap().visible);
    }

    #[test]
    fn test_world_editor_slack() {
        let settings = WorldSettings {
            name: "MySlack".to_string(),
            world_type: "slack".to_string(),
            slack_token: "xoxb-token".to_string(),
            slack_channel: "#general".to_string(),
            ..Default::default()
        };
        let def = create_world_editor_popup(&settings);
        let state = PopupState::new(def);

        // Slack fields should be visible
        assert!(state.field(WORLD_FIELD_SLACK_TOKEN).unwrap().visible);
        assert!(state.field(WORLD_FIELD_SLACK_CHANNEL).unwrap().visible);
        // MUD fields should be hidden
        assert!(!state.field(WORLD_FIELD_HOSTNAME).unwrap().visible);
    }
}
