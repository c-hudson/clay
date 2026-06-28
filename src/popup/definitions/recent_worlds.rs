//! Recent Worlds popup definition
//!
//! Lists the most recently active worlds (excluding the current one) so the
//! user can quickly jump back to a world they were working in.

use crate::popup::{
    Button, ButtonId, Field, FieldId, FieldKind, ListItem, ListItemStyle,
    PopupDefinition, PopupId, PopupLayout,
};
use crate::popup::definitions::connections::format_elapsed;

// Field IDs
pub const RECENT_FIELD_LIST: FieldId = FieldId(1);
pub const RECENT_FIELD_SPACER: FieldId = FieldId(99);

// Button IDs
pub const RECENT_BTN_OK: ButtonId = ButtonId(1);
pub const RECENT_BTN_CLOSE: ButtonId = ButtonId(2);

/// Information about a recently-active world for display in the switcher.
#[derive(Debug, Clone)]
pub struct RecentWorldInfo {
    /// World name (used as the list item id for switching)
    pub name: String,
    /// Seconds since last activity (None if never received/sent data)
    pub last_secs: Option<u64>,
}

/// Create the recent worlds popup definition.
///
/// `worlds` must already be sorted most-recent-first (lowest `last_secs` first)
/// and must NOT contain the current world.
pub fn create_recent_worlds_popup(worlds: &[RecentWorldInfo], visible_height: usize) -> PopupDefinition {
    if worlds.is_empty() {
        // Empty state: just a label and a Close button
        return PopupDefinition::new(PopupId("recent_worlds"), "Recent Worlds")
            .with_field(Field::new(
                RECENT_FIELD_LIST,
                "",
                FieldKind::label("No recently active worlds."),
            ))
            .with_field(Field::new(RECENT_FIELD_SPACER, "", FieldKind::label("")))
            .with_button(Button::new(RECENT_BTN_CLOSE, "Close").with_shortcut('C').with_tab_index(0))
            .with_layout(PopupLayout {
                label_width: 0,
                min_width: 40,
                max_width_percent: 60,
                center_horizontal: true,
                center_vertical: true,
                modal: true,
                buttons_right_align: true,
                blank_line_before_list: false,
                tab_buttons_only: false,
                anchor_bottom_left: false,
                anchor_x: 0,
            });
    }

    // Build list items: two columns — World name and Last Active relative time.
    let items: Vec<ListItem> = worlds
        .iter()
        .map(|w| ListItem {
            id: w.name.clone(),
            columns: vec![
                w.name.clone(),
                format_elapsed(w.last_secs),
            ],
            style: ListItemStyle {
                is_current: false,  // current world is never in this list
                is_connected: false,
                is_disabled: false,
            },
        })
        .collect();

    // Fixed column widths: world name up to 20 chars, time up to 12 chars.
    // These are intentionally fixed (not data-derived) so the popup size is stable.
    let column_widths = vec![20usize, 12usize];
    let headers: &[&str] = &["World", "Last Active"];

    PopupDefinition::new(PopupId("recent_worlds"), "Recent Worlds")
        .with_field(Field::new(
            RECENT_FIELD_LIST,
            "",
            FieldKind::list_with_headers_and_widths(items, visible_height, headers, column_widths),
        ))
        // Blank spacer line between list and buttons (no blank_line_before_buttons flag exists)
        .with_field(Field::new(RECENT_FIELD_SPACER, "", FieldKind::label("")))
        .with_button(Button::new(RECENT_BTN_OK, "OK").primary().with_shortcut('O'))
        .with_button(Button::new(RECENT_BTN_CLOSE, "Close").with_shortcut('C').with_tab_index(0))
        .with_layout(PopupLayout {
            label_width: 0,
            min_width: 40,
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::popup::PopupState;

    fn sample_worlds() -> Vec<RecentWorldInfo> {
        vec![
            RecentWorldInfo { name: "MUD1".to_string(), last_secs: Some(120) },
            RecentWorldInfo { name: "MUD2".to_string(), last_secs: Some(3600) },
            RecentWorldInfo { name: "MUD3".to_string(), last_secs: None },
        ]
    }

    #[test]
    fn test_recent_worlds_creation() {
        let worlds = sample_worlds();
        let def = create_recent_worlds_popup(&worlds, 6);
        let state = PopupState::new(def);

        assert_eq!(state.definition.id, PopupId("recent_worlds"));
        assert_eq!(state.definition.title, "Recent Worlds");

        // Should have list field + spacer field
        assert!(state.field(RECENT_FIELD_LIST).is_some());
        assert!(state.field(RECENT_FIELD_SPACER).is_some());

        // Should have OK and Close buttons (plus ? help button added by with_help if used)
        let btn_ids: Vec<ButtonId> = state.definition.buttons.iter().map(|b| b.id).collect();
        assert!(btn_ids.contains(&RECENT_BTN_OK));
        assert!(btn_ids.contains(&RECENT_BTN_CLOSE));

        // Check list has 3 items
        if let Some(field) = state.field(RECENT_FIELD_LIST) {
            if let FieldKind::List { items, .. } = &field.kind {
                assert_eq!(items.len(), 3);
                assert_eq!(items[0].id, "MUD1");
                assert_eq!(items[1].id, "MUD2");
                // Relative time column: 120s → "2m"
                assert_eq!(items[0].columns[1], "2m");
                // 3600s → "1h"
                assert_eq!(items[1].columns[1], "1h");
                // None → "-"
                assert_eq!(items[2].columns[1], "-");
            } else {
                panic!("RECENT_FIELD_LIST is not a List field");
            }
        }
    }

    #[test]
    fn test_recent_worlds_empty() {
        let def = create_recent_worlds_popup(&[], 1);
        let state = PopupState::new(def);

        // No list field in empty state — just a label
        if let Some(field) = state.field(RECENT_FIELD_LIST) {
            assert!(matches!(field.kind, FieldKind::Label { .. }));
        }

        // Only Close button (no OK in empty state)
        let has_ok = state.definition.buttons.iter().any(|b| b.id == RECENT_BTN_OK);
        let has_close = state.definition.buttons.iter().any(|b| b.id == RECENT_BTN_CLOSE);
        assert!(!has_ok, "Empty popup should not have OK button");
        assert!(has_close, "Empty popup should have Close button");
    }
}
