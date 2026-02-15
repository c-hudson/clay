//! Notes list popup definition
//!
//! Shows a list of worlds that have notes, allowing the user to open them in the editor.

use crate::popup::{
    Button, ButtonId, Field, FieldId, FieldKind, ListItem, ListItemStyle,
    PopupDefinition, PopupId, PopupLayout,
};

// Field IDs
pub const NOTES_FIELD_LIST: FieldId = FieldId(1);

// Button IDs
pub const NOTES_BTN_OPEN: ButtonId = ButtonId(1);
pub const NOTES_BTN_CANCEL: ButtonId = ButtonId(2);

/// Note info for the list
#[derive(Debug, Clone)]
pub struct NoteInfo {
    pub world_name: String,
    pub preview: String,
    pub is_current: bool,
}

/// Column headers for the notes list
pub const NOTES_LIST_HEADERS: &[&str] = &["World", "Preview"];

/// Create the notes list popup definition
pub fn create_notes_list_popup(notes: &[NoteInfo], visible_height: usize) -> PopupDefinition {
    let items: Vec<ListItem> = notes
        .iter()
        .map(|n| {
            let columns = vec![
                n.world_name.clone(),
                n.preview.clone(),
            ];

            ListItem {
                id: n.world_name.clone(),
                columns,
                style: ListItemStyle {
                    is_current: n.is_current,
                    is_connected: false,
                    is_disabled: false,
                },
            }
        })
        .collect();

    // Calculate column widths from headers and all items
    let num_columns = NOTES_LIST_HEADERS.len();
    let mut column_widths: Vec<usize> = NOTES_LIST_HEADERS.iter().map(|h| h.len()).collect();
    for item in &items {
        for (i, col) in item.columns.iter().enumerate() {
            if i < num_columns {
                column_widths[i] = column_widths[i].max(col.len());
            }
        }
    }

    PopupDefinition::new(PopupId("notes_list"), "World Notes")
        .with_field(Field::new(
            NOTES_FIELD_LIST,
            "",
            FieldKind::list_with_headers_and_widths(items, visible_height, NOTES_LIST_HEADERS, column_widths),
        ))
        .with_button(Button::new(NOTES_BTN_CANCEL, "Cancel").with_shortcut('C').with_tab_index(1))
        .with_button(Button::new(NOTES_BTN_OPEN, "Open").primary().with_shortcut('O').with_tab_index(0))
        .with_layout(PopupLayout {
            label_width: 8,
            min_width: 50,
            max_width_percent: 80,
            center_horizontal: true,
            center_vertical: false,
            modal: true,
            buttons_right_align: true,
            blank_line_before_list: true,
            tab_buttons_only: false,
        })
}
