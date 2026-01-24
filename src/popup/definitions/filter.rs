//! Filter popup definition
//!
//! A small popup for filtering output lines by text pattern.

use crate::popup::{
    Field, FieldId, FieldKind, PopupDefinition, PopupId, PopupLayout,
};

// Field IDs
pub const FILTER_FIELD_TEXT: FieldId = FieldId(1);

/// Create the filter popup definition
pub fn create_filter_popup() -> PopupDefinition {
    PopupDefinition::new(PopupId("filter"), "Filter")
        .with_field(Field::new(
            FILTER_FIELD_TEXT,
            "",
            FieldKind::text_with_placeholder("", "Enter filter text..."),
        ))
        .with_layout(PopupLayout {
            label_width: 0,
            min_width: 30,
            max_width_percent: 40,
            center_horizontal: false,  // Positioned in upper right
            center_vertical: false,
            modal: false,  // Non-modal, shows filtered output behind
            buttons_right_align: false,
            blank_line_before_list: false,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::popup::PopupState;

    #[test]
    fn test_filter_popup_creation() {
        let def = create_filter_popup();
        let state = PopupState::new(def);

        assert_eq!(state.definition.id, PopupId("filter"));
        assert_eq!(state.definition.title, "Filter");
        assert!(state.field(FILTER_FIELD_TEXT).is_some());
    }

    #[test]
    fn test_filter_text_editing() {
        let def = create_filter_popup();
        let mut state = PopupState::new(def);
        state.open();

        // Start editing the filter text
        state.start_edit();
        state.insert_char('t');
        state.insert_char('e');
        state.insert_char('s');
        state.insert_char('t');
        state.commit_edit();

        assert_eq!(state.get_text(FILTER_FIELD_TEXT), Some("test"));
    }
}
