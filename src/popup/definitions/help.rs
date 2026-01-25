//! Help popup definition
//!
//! Displays help content with keyboard shortcuts and commands.

use crate::popup::{
    Button, ButtonId, Field, FieldId, FieldKind, PopupDefinition, PopupId, PopupLayout,
};

// Field IDs
pub const HELP_FIELD_CONTENT: FieldId = FieldId(1);

// Button IDs
pub const HELP_BTN_OK: ButtonId = ButtonId(1);

/// Help content lines
pub const HELP_LINES: &[&str] = &[
    "Commands:",
    "  /help                      Show this help",
    "  /disconnect (or /dc)       Disconnect from server",
    "  /connect [host port [ssl]] Connect to server",
    "  /send [-W] [-w<world>] [-n] <text>",
    "                             Send text to world(s)",
    "  /worlds                    Open world selector",
    "  /worlds <name>             Connect to or create world",
    "  /worlds -e [name]          Edit world settings",
    "  /worlds -l <name>          Connect without auto-login",
    "  /connections (or /l)       List connected worlds",
    "  /keepalive                 Show keepalive settings",
    "  /actions [world]           Open actions editor",
    "  /gag <pattern>             Gag lines matching pattern",
    "  /setup                     Open global settings",
    "  /web                       Open web/WebSocket settings",
    "  /menu                      Open menu popup",
    "  /flush                     Clear output buffer",
    "  /reload                    Hot reload binary",
    "  /quit                      Exit client",
    "",
    "World Switching:",
    "  Up/Down                    Switch worlds",
    "",
    "Input:",
    "  Left/Right, Ctrl+B/F       Move cursor",
    "  Ctrl+Up/Down               Resize input area",
    "  Ctrl+U                     Clear input",
    "  Ctrl+W                     Delete word",
    "  Ctrl+P/N                   Command history",
    "  Ctrl+Q                     Spell suggestions",
    "  Home/End                   Jump to start/end",
    "",
    "Output:",
    "  PageUp/PageDown            Scroll output",
    "  Tab                        Release one screenful",
    "  Alt+j                      Jump to end",
    "",
    "Display:",
    "  F1                         Show this help",
    "  F2                         Toggle MUD tag display",
    "  F4                         Filter output",
    "  F8                         Highlight action matches",
    "",
    "General:",
    "  Ctrl+C (twice)             Quit",
    "  Ctrl+L                     Redraw screen",
    "  Ctrl+R                     Hot reload",
    "  Ctrl+Z                     Suspend",
];

/// Create the help popup definition
pub fn create_help_popup() -> PopupDefinition {
    PopupDefinition::new(PopupId("help"), "Help")
        .with_field(Field::new(
            HELP_FIELD_CONTENT,
            "",
            // Use large visible_height - will be capped by terminal size
            FieldKind::scrollable_content_static(HELP_LINES, 100),
        ))
        .with_button(Button::new(HELP_BTN_OK, "Ok").primary().with_shortcut('O'))
        .with_layout(PopupLayout {
            label_width: 0,
            min_width: 1000, // Large value to ensure 90% width is used
            max_width_percent: 90,
            center_horizontal: true,
            center_vertical: false, // Position at top, not centered
            modal: true,
            buttons_right_align: false,
            blank_line_before_list: false,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::popup::PopupState;

    #[test]
    fn test_help_popup_creation() {
        let def = create_help_popup();
        let state = PopupState::new(def);

        assert_eq!(state.definition.id, PopupId("help"));
        assert_eq!(state.definition.title, "Help");
        assert!(!state.definition.buttons.is_empty());
    }
}
