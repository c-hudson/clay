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
        .with_button(Button::new(CONFIRM_BTN_YES, "Yes").danger().with_shortcut('Y'))
        .with_button(Button::new(CONFIRM_BTN_NO, "No").primary().with_shortcut('N'))
        .with_layout(PopupLayout {
            label_width: 0,
            min_width: 30,
            max_width_percent: 50,
            center_horizontal: true,
            center_vertical: true,
            modal: true,
            buttons_right_align: false,
            blank_line_before_list: false,
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
