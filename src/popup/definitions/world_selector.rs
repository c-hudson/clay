//! World selector popup definition
//!
//! Shows a list of all worlds with filter and selection.

use crate::popup::{
    Button, ButtonId, Field, FieldId, FieldKind, ListItem, ListItemStyle,
    PopupDefinition, PopupId, PopupLayout,
};

// Field IDs
pub const SELECTOR_FIELD_FILTER: FieldId = FieldId(1);
pub const SELECTOR_FIELD_LIST: FieldId = FieldId(2);

// Button IDs
pub const SELECTOR_BTN_ADD: ButtonId = ButtonId(1);
pub const SELECTOR_BTN_EDIT: ButtonId = ButtonId(2);
pub const SELECTOR_BTN_DELETE: ButtonId = ButtonId(3);
pub const SELECTOR_BTN_CONNECT: ButtonId = ButtonId(4);
pub const SELECTOR_BTN_CANCEL: ButtonId = ButtonId(5);

/// World info for the selector
#[derive(Debug, Clone)]
pub struct WorldInfo {
    pub name: String,
    pub hostname: String,
    pub port: String,
    pub user: String,
    pub is_connected: bool,
    pub is_current: bool,
}

/// Column headers for the world list
pub const WORLD_LIST_HEADERS: &[&str] = &["World", "Hostname", "Port", "User"];

/// Create the world selector popup definition
pub fn create_world_selector_popup(worlds: &[WorldInfo], visible_height: usize) -> PopupDefinition {
    let items: Vec<ListItem> = worlds
        .iter()
        .map(|w| {
            // Columns: World, Hostname, Port, User
            let columns = vec![
                w.name.clone(),
                w.hostname.clone(),
                w.port.clone(),
                w.user.clone(),
            ];

            ListItem {
                id: w.name.clone(),
                columns,
                style: ListItemStyle {
                    is_current: w.is_current,
                    is_connected: w.is_connected,
                    is_disabled: false,
                },
            }
        })
        .collect();

    // Calculate column widths from headers and all items (so they don't change when filtering)
    let num_columns = WORLD_LIST_HEADERS.len();
    let mut column_widths: Vec<usize> = WORLD_LIST_HEADERS.iter().map(|h| h.len()).collect();
    for item in &items {
        for (i, col) in item.columns.iter().enumerate() {
            if i < num_columns {
                column_widths[i] = column_widths[i].max(col.len());
            }
        }
    }

    PopupDefinition::new(PopupId("world_selector"), "World Selector")
        .with_field(Field::new(
            SELECTOR_FIELD_FILTER,
            "Filter",
            FieldKind::text_with_placeholder("", "Type to filter..."),
        ).with_shortcut('F'))
        .with_field(Field::new(
            SELECTOR_FIELD_LIST,
            "",
            FieldKind::list_with_headers_and_widths(items, visible_height, WORLD_LIST_HEADERS, column_widths),
        ))
        .with_button(Button::new(SELECTOR_BTN_ADD, "Add").with_shortcut('A'))
        .with_button(Button::new(SELECTOR_BTN_EDIT, "Edit").with_shortcut('E'))
        .with_button(Button::new(SELECTOR_BTN_DELETE, "Delete").danger().with_shortcut('D').left_align())
        .with_button(Button::new(SELECTOR_BTN_CONNECT, "Connect").primary().with_shortcut('O'))
        .with_button(Button::new(SELECTOR_BTN_CANCEL, "Cancel").with_shortcut('C'))
        .with_layout(PopupLayout {
            label_width: 8,
            min_width: 60,
            max_width_percent: 90,
            center_horizontal: true,
            center_vertical: false,  // Top-aligned for better content fitting
            modal: true,
            buttons_right_align: true,
            blank_line_before_list: true,
        })
}

/// Filter the world list based on filter text
pub fn filter_worlds(all_worlds: &[WorldInfo], filter: &str) -> Vec<WorldInfo> {
    if filter.is_empty() {
        return all_worlds.to_vec();
    }

    let filter_lower = filter.to_lowercase();
    all_worlds
        .iter()
        .filter(|w| {
            w.name.to_lowercase().contains(&filter_lower)
                || w.hostname.to_lowercase().contains(&filter_lower)
                || w.user.to_lowercase().contains(&filter_lower)
        })
        .cloned()
        .collect()
}

/// Update the list field with filtered worlds
pub fn update_world_list(state: &mut crate::popup::PopupState, worlds: &[WorldInfo]) {
    if let Some(field) = state.field_mut(SELECTOR_FIELD_LIST) {
        if let FieldKind::List { items, selected_index, scroll_offset, .. } = &mut field.kind {
            let new_items: Vec<ListItem> = worlds
                .iter()
                .map(|w| {
                    // Columns: World, Hostname, Port, User
                    let columns = vec![
                        w.name.clone(),
                        w.hostname.clone(),
                        w.port.clone(),
                        w.user.clone(),
                    ];

                    ListItem {
                        id: w.name.clone(),
                        columns,
                        style: ListItemStyle {
                            is_current: w.is_current,
                            is_connected: w.is_connected,
                            is_disabled: false,
                        },
                    }
                })
                .collect();

            *items = new_items;
            // Reset selection if needed
            if *selected_index >= items.len() {
                *selected_index = items.len().saturating_sub(1);
            }
            if *scroll_offset > *selected_index {
                *scroll_offset = *selected_index;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::popup::PopupState;

    fn sample_worlds() -> Vec<WorldInfo> {
        vec![
            WorldInfo {
                name: "TestMUD".to_string(),
                hostname: "mud.example.com".to_string(),
                port: "4000".to_string(),
                user: "player1".to_string(),
                is_connected: true,
                is_current: true,
            },
            WorldInfo {
                name: "AnotherMUD".to_string(),
                hostname: "another.mud.com".to_string(),
                port: "5000".to_string(),
                user: "player2".to_string(),
                is_connected: false,
                is_current: false,
            },
        ]
    }

    #[test]
    fn test_world_selector_creation() {
        let worlds = sample_worlds();
        let def = create_world_selector_popup(&worlds, 10);
        let state = PopupState::new(def);

        assert_eq!(state.definition.id, PopupId("world_selector"));
        assert_eq!(state.definition.title, "World Selector");
        assert_eq!(state.definition.buttons.len(), 5);
    }

    #[test]
    fn test_filter_worlds() {
        let worlds = sample_worlds();

        let filtered = filter_worlds(&worlds, "Test");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "TestMUD");

        let filtered = filter_worlds(&worlds, "");
        assert_eq!(filtered.len(), 2);

        let filtered = filter_worlds(&worlds, "nonexistent");
        assert!(filtered.is_empty());
    }
}
