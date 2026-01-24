//! Menu popup definition
//!
//! A popup that shows a list of menu items for navigation.

use crate::popup::{
    Button, ButtonId, Field, FieldId, FieldKind, ListItem, ListItemStyle,
    PopupDefinition, PopupId, PopupLayout,
};

// Field IDs
pub const MENU_FIELD_LIST: FieldId = FieldId(1);

// Button IDs
pub const MENU_BTN_CANCEL: ButtonId = ButtonId(1);

/// Menu item enumeration
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MenuItem {
    Help,
    Setup,
    WebSettings,
    Actions,
    WorldSelector,
    ConnectedWorlds,
}

impl MenuItem {
    pub fn all() -> &'static [MenuItem] {
        &[
            MenuItem::Help,
            MenuItem::Setup,
            MenuItem::WebSettings,
            MenuItem::Actions,
            MenuItem::WorldSelector,
            MenuItem::ConnectedWorlds,
        ]
    }

    pub fn label(&self) -> &'static str {
        match self {
            MenuItem::Help => "Help",
            MenuItem::Setup => "Settings",
            MenuItem::WebSettings => "Web Settings",
            MenuItem::Actions => "Actions",
            MenuItem::WorldSelector => "World Selector",
            MenuItem::ConnectedWorlds => "Connected Worlds",
        }
    }

    pub fn command(&self) -> &'static str {
        match self {
            MenuItem::Help => "/help",
            MenuItem::Setup => "/setup",
            MenuItem::WebSettings => "/web",
            MenuItem::Actions => "/actions",
            MenuItem::WorldSelector => "/worlds",
            MenuItem::ConnectedWorlds => "/connections",
        }
    }
}

/// Create the menu popup definition
pub fn create_menu_popup() -> PopupDefinition {
    let items: Vec<ListItem> = MenuItem::all()
        .iter()
        .map(|item| ListItem {
            id: item.command().to_string(),
            columns: vec![item.label().to_string()],
            style: ListItemStyle::default(),
        })
        .collect();

    PopupDefinition::new(PopupId("menu"), "Menu")
        .with_field(Field::new(
            MENU_FIELD_LIST,
            "",
            FieldKind::list(items, 10),
        ))
        .with_button(Button::new(MENU_BTN_CANCEL, "Cancel").with_shortcut('C'))
        .with_layout(PopupLayout {
            label_width: 0,
            min_width: 25,
            max_width_percent: 40,
            center_horizontal: true,
            center_vertical: true,
            modal: true,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::popup::PopupState;

    #[test]
    fn test_menu_popup_creation() {
        let def = create_menu_popup();
        let state = PopupState::new(def);

        assert_eq!(state.definition.id, PopupId("menu"));
        assert_eq!(state.definition.title, "Menu");
    }

    #[test]
    fn test_menu_items() {
        assert_eq!(MenuItem::all().len(), 6);
        assert_eq!(MenuItem::Help.label(), "Help");
        assert_eq!(MenuItem::Help.command(), "/help");
    }
}
