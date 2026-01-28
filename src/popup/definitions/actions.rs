//! Actions popup definition
//!
//! Allows viewing and editing action triggers.

use crate::popup::{
    Button, ButtonId, Field, FieldId, FieldKind, ListItem, ListItemStyle,
    PopupDefinition, PopupId, PopupLayout, SelectOption,
};

// ============================================================================
// Actions List View
// ============================================================================

// Field IDs - List view
pub const ACTIONS_FIELD_FILTER: FieldId = FieldId(1);
pub const ACTIONS_FIELD_LIST: FieldId = FieldId(2);

// Button IDs - List view
pub const ACTIONS_BTN_ADD: ButtonId = ButtonId(1);
pub const ACTIONS_BTN_EDIT: ButtonId = ButtonId(2);
pub const ACTIONS_BTN_DELETE: ButtonId = ButtonId(3);
pub const ACTIONS_BTN_CANCEL: ButtonId = ButtonId(4);

// ============================================================================
// Action Editor View
// ============================================================================

// Field IDs - Editor view
pub const EDITOR_FIELD_NAME: FieldId = FieldId(10);
pub const EDITOR_FIELD_WORLD: FieldId = FieldId(11);
pub const EDITOR_FIELD_MATCH_TYPE: FieldId = FieldId(12);
pub const EDITOR_FIELD_PATTERN: FieldId = FieldId(13);
pub const EDITOR_FIELD_COMMAND: FieldId = FieldId(14);
pub const EDITOR_FIELD_ENABLED: FieldId = FieldId(15);

// Button IDs - Editor view
pub const EDITOR_BTN_SAVE: ButtonId = ButtonId(10);
pub const EDITOR_BTN_CANCEL: ButtonId = ButtonId(11);

/// Action info for display
#[derive(Debug, Clone)]
pub struct ActionInfo {
    pub name: String,
    pub world: String,
    pub pattern: String,
    pub enabled: bool,
    pub index: usize,
}

/// Match type options
pub fn match_type_options() -> Vec<SelectOption> {
    vec![
        SelectOption::new("regexp", "Regexp"),
        SelectOption::new("wildcard", "Wildcard"),
    ]
}

/// Create the actions list popup definition
pub fn create_actions_list_popup(actions: &[ActionInfo], visible_height: usize) -> PopupDefinition {
    // Sort alphabetically by action name (case-insensitive)
    let mut sorted_actions: Vec<&ActionInfo> = actions.iter().collect();
    sorted_actions.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    let items: Vec<ListItem> = sorted_actions
        .iter()
        .map(|a| {
            // Format: "[✓] name   world   pattern" (or "[x]" on Windows)
            #[cfg(not(windows))]
            let status = if a.enabled { "[✓]" } else { "[ ]" };
            #[cfg(windows)]
            let status = if a.enabled { "[x]" } else { "[ ]" };
            let world_part = a.world.clone();
            let pattern_preview = if a.pattern.len() > 30 {
                format!("{}...", &a.pattern[..27])
            } else {
                a.pattern.clone()
            };

            let columns = vec![
                format!("{} {}", status, a.name),
                world_part,
                pattern_preview,
            ];

            ListItem {
                id: a.index.to_string(),
                columns,
                style: ListItemStyle {
                    is_disabled: !a.enabled,
                    ..Default::default()
                },
            }
        })
        .collect();

    PopupDefinition::new(PopupId("actions_list"), "Actions")
        .with_field(Field::new(
            ACTIONS_FIELD_FILTER,
            "",
            FieldKind::text_with_placeholder("", "Filter Actions..."),
        ))
        .with_field(Field::new(
            ACTIONS_FIELD_LIST,
            "",
            FieldKind::list_with_headers_and_widths(
                items,
                visible_height,
                &["Name", "World", "Pattern"],
                vec![180, 100, 220],
            ),
        ))
        .with_button(Button::new(ACTIONS_BTN_DELETE, "Delete").danger().with_shortcut('D').left_align())
        .with_button(Button::new(ACTIONS_BTN_ADD, "Add").with_shortcut('A'))
        .with_button(Button::new(ACTIONS_BTN_EDIT, "Edit").with_shortcut('E'))
        .with_button(Button::new(ACTIONS_BTN_CANCEL, "Ok").primary().with_shortcut('O'))
        .with_layout(PopupLayout {
            label_width: 8,
            min_width: 550,  // Accommodate columns (180+100+220) plus padding
            max_width_percent: 85,
            center_horizontal: true,
            center_vertical: true,
            modal: true,
            buttons_right_align: true,
            blank_line_before_list: false,
        })
}

/// Action settings for the editor
#[derive(Debug, Clone, Default)]
pub struct ActionSettings {
    pub name: String,
    pub world: String,
    pub match_type: String,
    pub pattern: String,
    pub command: String,
    pub enabled: bool,
}

/// Create the action editor popup definition
pub fn create_action_editor_popup(settings: &ActionSettings, is_new: bool) -> PopupDefinition {
    let match_type_idx = if settings.match_type == "wildcard" { 1 } else { 0 };

    let title = if is_new { "New Action" } else { "Edit Action" };

    PopupDefinition::new(PopupId("action_editor"), title)
        .with_field(Field::new(
            EDITOR_FIELD_NAME,
            "Name",
            FieldKind::text(&settings.name),
        ))
        .with_field(Field::new(
            EDITOR_FIELD_WORLD,
            "World",
            FieldKind::text_with_placeholder(&settings.world, "(all worlds)"),
        ))
        .with_field(Field::new(
            EDITOR_FIELD_MATCH_TYPE,
            "Match Type",
            FieldKind::select(match_type_options(), match_type_idx),
        ))
        .with_field(Field::new(
            EDITOR_FIELD_PATTERN,
            "Pattern",
            FieldKind::text(&settings.pattern),
        ))
        .with_field(Field::new(
            EDITOR_FIELD_COMMAND,
            "Command",
            FieldKind::multiline(&settings.command, 3),
        ))
        .with_field(Field::new(
            EDITOR_FIELD_ENABLED,
            "Enabled",
            FieldKind::toggle(settings.enabled),
        ))
        .with_button(Button::new(EDITOR_BTN_SAVE, "Save").primary().with_shortcut('S'))
        .with_button(Button::new(EDITOR_BTN_CANCEL, "Cancel").with_shortcut('C'))
        .with_layout(PopupLayout {
            label_width: 12,
            min_width: 60,
            max_width_percent: 90,
            center_horizontal: true,
            center_vertical: true,
            modal: true,
            buttons_right_align: true,
            blank_line_before_list: false,
        })
}

/// Filter actions based on filter text
pub fn filter_actions(all_actions: &[ActionInfo], filter: &str) -> Vec<ActionInfo> {
    if filter.is_empty() {
        return all_actions.to_vec();
    }

    let filter_lower = filter.to_lowercase();
    all_actions
        .iter()
        .filter(|a| {
            a.name.to_lowercase().contains(&filter_lower)
                || a.world.to_lowercase().contains(&filter_lower)
                || a.pattern.to_lowercase().contains(&filter_lower)
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::popup::PopupState;

    fn sample_actions() -> Vec<ActionInfo> {
        vec![
            ActionInfo {
                name: "auto_heal".to_string(),
                world: "TestMUD".to_string(),
                pattern: "^You are bleeding".to_string(),
                enabled: true,
                index: 0,
            },
            ActionInfo {
                name: "highlight_says".to_string(),
                world: "".to_string(),
                pattern: "* says *".to_string(),
                enabled: false,
                index: 1,
            },
        ]
    }

    #[test]
    fn test_actions_list_creation() {
        let actions = sample_actions();
        let def = create_actions_list_popup(&actions, 10);
        let state = PopupState::new(def);

        assert_eq!(state.definition.id, PopupId("actions_list"));
        assert_eq!(state.definition.title, "Actions");
    }

    #[test]
    fn test_action_editor_creation() {
        let settings = ActionSettings {
            name: "test_action".to_string(),
            pattern: "test pattern".to_string(),
            command: "say hello".to_string(),
            enabled: true,
            ..Default::default()
        };
        let def = create_action_editor_popup(&settings, false);
        let state = PopupState::new(def);

        assert_eq!(state.definition.id, PopupId("action_editor"));
        assert_eq!(state.definition.title, "Edit Action");
        assert_eq!(state.get_text(EDITOR_FIELD_NAME), Some("test_action"));
    }

    #[test]
    fn test_filter_actions() {
        let actions = sample_actions();

        let filtered = filter_actions(&actions, "heal");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "auto_heal");

        let filtered = filter_actions(&actions, "");
        assert_eq!(filtered.len(), 2);
    }
}
