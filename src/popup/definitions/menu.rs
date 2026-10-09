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

/// What a menu item opens. `/menu` is the console's: every window or popup the console
/// can open without being told anything more (`/worlds`, but not `/worlds -e <name>`).
/// The web and GUI clients have their menu bar instead.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MenuItem {
    Help,
    Setup,
    WebSettings,
    Actions,
    WorldSelector,
    RecentWorlds,
    Notes,
    NotesList,
    ImportSettings,
    EmojiPicker,
    FilterOutput,
    SearchOutput,
}

impl MenuItem {
    /// The items, in order. The notes editor, its list and Import are the master
    /// console's own (the remote console has none of them).
    pub fn all(is_master: bool) -> Vec<MenuItem> {
        use MenuItem::*;
        [Help, Setup, WebSettings, Actions, WorldSelector, RecentWorlds, Notes, NotesList, ImportSettings, EmojiPicker, FilterOutput, SearchOutput]
            .into_iter()
            .filter(|i| is_master || !matches!(i, Notes | NotesList | ImportSettings))
            .collect()
    }

    pub fn label(&self) -> &'static str {
        match self {
            MenuItem::Help => "Help",
            MenuItem::Setup => "Settings",
            MenuItem::WebSettings => "Web Settings",
            MenuItem::Actions => "Actions",
            MenuItem::WorldSelector => "World Selector",
            MenuItem::RecentWorlds => "Recent Worlds",
            MenuItem::Notes => "Notes",
            MenuItem::NotesList => "Notes List",
            MenuItem::ImportSettings => "Import Settings",
            MenuItem::EmojiPicker => "Emoji Picker",
            MenuItem::FilterOutput => "Filter Output",
            MenuItem::SearchOutput => "Search Output",
        }
    }

    /// What choosing it runs: a command, or a key's action as `action:<id>` (the popups
    /// that only a key opens).
    pub fn command(&self) -> &'static str {
        match self {
            MenuItem::Help => "/help",
            MenuItem::Setup => "/setup",
            MenuItem::WebSettings => "/web",
            MenuItem::Actions => "/actions",
            MenuItem::WorldSelector => "/worlds",
            MenuItem::RecentWorlds => "action:recent_worlds",
            MenuItem::Notes => "/note",
            MenuItem::NotesList => "/note -l",
            MenuItem::ImportSettings => "/import",
            MenuItem::EmojiPicker => "action:emoji_picker",
            MenuItem::FilterOutput => "action:filter_popup",
            MenuItem::SearchOutput => "action:search_popup",
        }
    }
}

/// A menu item's key action (`action:<id>`), if it is one.
pub fn menu_action(command: &str) -> Option<&str> {
    command.strip_prefix("action:")
}

/// Create the menu popup definition
pub fn create_menu_popup(is_master: bool) -> PopupDefinition {
    let all = MenuItem::all(is_master);
    let rows = all.len();
    let items: Vec<ListItem> = all
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
            FieldKind::list(items, rows),
        ))
        .with_button(Button::new(MENU_BTN_CANCEL, "Cancel").with_shortcut('C'))
        .with_layout(PopupLayout {
            label_width: 0,
            min_width: 25,
            max_width_percent: 40,
            center_horizontal: true,
            center_vertical: true,
            modal: true,
            buttons_right_align: false,
            blank_line_before_list: false,
            tab_buttons_only: false,
            anchor_bottom_left: false,
            anchor_x: 0,
        })
        .with_help(menu_help_text())
}


/// Help text for the Menu popup
fn menu_help_text() -> Vec<String> {
    vec![
        "Menu - every window and popup in one list",
        "",
        "Select an item and press Enter to open it.",
        "",
        "  Help             - Keyboard shortcuts and commands  (/help)",
        "  Settings         - Global settings                  (/setup)",
        "  Web Settings     - Web server and remote access     (/web)",
        "  Actions          - Triggers and automation          (/actions)",
        "  World Selector   - Browse and connect to worlds     (/worlds)",
        "  Recent Worlds    - Worlds you were in lately        (key)",
        "  Notes            - This world's notes               (/note)",
        "  Notes List       - Every world's notes              (/note -l)",
        "  Import Settings  - From another Clay                (/import)",
        "  Emoji Picker     - Pick an emoji to type            (Esc-e)",
        "  Filter Output    - Show only matching lines         (F4)",
        "  Search Output    - Find text in the output          (key)",
        "",
        "/menu is the console's. The web and GUI interfaces have their menu bar.",
    ].into_iter().map(|s| s.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::popup::PopupState;

    #[test]
    fn test_menu_popup_creation() {
        let def = create_menu_popup(true);
        let state = PopupState::new(def);

        assert_eq!(state.definition.id, PopupId("menu"));
        assert_eq!(state.definition.title, "Menu");
    }

    #[test]
    fn test_menu_items() {
        assert_eq!(MenuItem::all(true).len(), 12);
        assert_eq!(MenuItem::Help.label(), "Help");
        assert_eq!(MenuItem::Help.command(), "/help");
        // The remote console has no notes editor or Import.
        let remote = MenuItem::all(false);
        assert!(!remote.contains(&MenuItem::Notes) && !remote.contains(&MenuItem::ImportSettings));
        assert_eq!(menu_action(MenuItem::EmojiPicker.command()), Some("emoji_picker"));
        assert_eq!(menu_action("/help"), None);
    }

    #[test]
    fn every_menu_command_parses_to_what_it_opens() {
        // Each command item is a no-argument command the console knows.
        for item in MenuItem::all(true) {
            let cmd = item.command();
            if menu_action(cmd).is_some() {
                continue;
            }
            assert!(!matches!(crate::parse_command(cmd), crate::Command::Unknown { .. } | crate::Command::NotACommand { .. } | crate::Command::ActionCommand { .. }), "{cmd}");
        }
    }
}
