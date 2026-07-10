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
pub const EDITOR_FIELD_MATCH_TYPE: FieldId = FieldId(12);  // Single action-level match type
pub const EDITOR_FIELD_PATTERNS: FieldId = FieldId(13);    // EditableList of pattern strings
pub const EDITOR_FIELD_COMMAND: FieldId = FieldId(14);
pub const EDITOR_FIELD_ENABLED: FieldId = FieldId(15);
pub const EDITOR_FIELD_STARTUP: FieldId = FieldId(16);

// Button IDs - Editor view
pub const EDITOR_BTN_SAVE: ButtonId = ButtonId(10);
pub const EDITOR_BTN_CANCEL: ButtonId = ButtonId(11);
pub const EDITOR_BTN_DELETE: ButtonId = ButtonId(12);

/// Action info for display
#[derive(Debug, Clone)]
pub struct ActionInfo {
    pub name: String,
    pub world: String,
    pub pattern: String,  // First pattern preview for list display
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
                let mut end = 27;
                while end > 0 && !a.pattern.is_char_boundary(end) {
                    end -= 1;
                }
                format!("{}...", &a.pattern[..end])
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
        ).with_shortcut('F').search())
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
            tab_buttons_only: false,
            anchor_bottom_left: false,
            anchor_x: 0,
        })
        .with_help(actions_list_help_text())
}

/// Help text for the Actions List popup
fn actions_list_help_text() -> Vec<String> {
    vec![
        "Actions - Triggers and Automation",
        "",
        "Actions automatically respond to MUD output. When",
        "text from the MUD matches an action's pattern, the",
        "action's command is executed.",
        "",
        "List columns:",
        "  Name    - Action name (with enabled/disabled status)",
        "  World   - Which world this action applies to",
        "            (blank = all worlds)",
        "  Pattern - The text pattern to match",
        "",
        "Navigation:",
        "  Up/Down   Navigate the action list",
        "  Enter     Edit the selected action",
        "  Space     Toggle enabled/disabled",
        "  Tab       Cycle between buttons",
        "  Esc       Close this popup",
        "",
        "Buttons:",
        "  Add (A)    Create a new action",
        "  Edit (E)   Edit the selected action",
        "  Delete (D) Remove the selected action",
        "  Ok (O)     Close this popup",
        "",
        "Use the filter field at the top to search actions",
        "by name, world, or pattern.",
    ].into_iter().map(|s| s.to_string()).collect()
}

/// Action settings for the editor.
/// `patterns` holds the list of pattern strings (all sharing the same `match_type`).
/// Displayed in a scrollable EditableList with max 4 visible rows.
#[derive(Debug, Clone, Default)]
pub struct ActionSettings {
    pub name: String,
    pub world: String,
    /// Action-level match type ("regexp" or "wildcard")
    pub match_type: String,
    /// Pattern strings (action-level match type applies to all)
    pub patterns: Vec<String>,
    pub command: String,
    pub enabled: bool,
    pub startup: bool,
}

/// Create the action editor popup definition (TUI — editable pattern list, max 4 visible)
pub fn create_action_editor_popup(settings: &ActionSettings, is_new: bool) -> PopupDefinition {
    let mt_idx = |s: &str| if s == "wildcard" { 1 } else { 0 };

    let title = if is_new { "New Action" } else { "Edit Action" };

    // Build pattern items: always at least one empty row so the user has somewhere to type
    let mut pattern_items: Vec<String> = settings.patterns.clone();
    if pattern_items.is_empty() {
        pattern_items.push(String::new());
    }

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
        // Single action-level match type
        .with_field(Field::new(
            EDITOR_FIELD_MATCH_TYPE,
            "Match Type",
            FieldKind::select(match_type_options(), mt_idx(&settings.match_type)),
        ))
        // Full-width scrollable editable pattern list (max 4 visible)
        .with_field(Field::new(
            EDITOR_FIELD_PATTERNS,
            "Patterns",
            FieldKind::editable_list(pattern_items, 4),
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
        .with_field(Field::new(
            EDITOR_FIELD_STARTUP,
            "Startup",
            FieldKind::toggle(settings.startup),
        ))
        .with_button_if(!is_new, Button::new(EDITOR_BTN_DELETE, "Delete").danger().with_shortcut('D').left_align())
        .with_button(Button::new(EDITOR_BTN_CANCEL, "Cancel").with_shortcut('C'))
        .with_button(Button::new(EDITOR_BTN_SAVE, "Save").primary().with_shortcut('S'))
        .with_layout(PopupLayout {
            label_width: 12,
            min_width: 70,
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
        .with_help(action_editor_help_text())
}

/// Help text for the Action Editor popup
fn action_editor_help_text() -> Vec<String> {
    vec![
        "Action Editor - Configure a Trigger",
        "",
        "Name: A unique name for this action. Can be invoked",
        "  manually with /name (e.g. /heal runs action 'heal').",
        "",
        "World: Which world this action applies to. Leave",
        "  blank to match output from any world.",
        "",
        "Match Type: How all patterns are interpreted.",
        "  Regexp   - Regular expression (e.g. ^You are (\\w+))",
        "  Wildcard - Simple wildcards (* matches anything)",
        "",
        "Patterns: Text patterns to match against MUD output.",
        "  The action fires when ANY pattern matches; the first",
        "  matching pattern (in order) supplies $1..$9 captures.",
        "  Leave all patterns empty for actions invoked only",
        "  manually (by typing /actionname).",
        "  Up/Down arrows navigate between patterns.",
        "  Type to edit the selected pattern in place.",
        "  Clear a pattern to remove it (pruned on Save).",
        "  Down past the last filled row adds a new row.",
        "",
        "Command: What to execute when the pattern matches.",
        "  Multiple commands separated by semicolons (;).",
        "  Use $1-$9 for captured groups from the pattern.",
        "  Special commands in the command field:",
        "    /gag          - Hide the matched line",
        "    /notify msg   - Send a push notification",
        "    /echo msg     - Display a local message",
        "",
        "Enabled: Whether this action is active.",
        "",
        "Startup: Run this action's command when Clay starts",
        "  or hot-reloads (useful for initialization scripts).",
    ].into_iter().map(|s| s.to_string()).collect()
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
            match_type: "regexp".to_string(),
            patterns: vec!["test pattern".to_string()],
            command: "say hello".to_string(),
            enabled: true,
            ..Default::default()
        };
        let def = create_action_editor_popup(&settings, false);
        let state = PopupState::new(def);

        assert_eq!(state.definition.id, PopupId("action_editor"));
        assert_eq!(state.definition.title, "Edit Action");
        assert_eq!(state.get_text(EDITOR_FIELD_NAME), Some("test_action"));
        // EditableList selected row 0 should be the first pattern
        assert_eq!(state.get_text(EDITOR_FIELD_PATTERNS), Some("test pattern"));
    }

    #[test]
    fn test_action_editor_multi_pattern() {
        let settings = ActionSettings {
            name: "multi_test".to_string(),
            match_type: "regexp".to_string(),
            patterns: vec![
                "^pattern one".to_string(),
                "pattern two".to_string(),
            ],
            command: "say hi".to_string(),
            enabled: true,
            ..Default::default()
        };
        let def = create_action_editor_popup(&settings, false);
        let state = PopupState::new(def);

        // First item is selected by default
        assert_eq!(state.get_text(EDITOR_FIELD_PATTERNS), Some("^pattern one"));
        // All items accessible via get_items
        let items = state.field(EDITOR_FIELD_PATTERNS)
            .and_then(|f| f.kind.get_items())
            .map(|s| s.to_vec())
            .unwrap_or_default();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0], "^pattern one");
        assert_eq!(items[1], "pattern two");
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
