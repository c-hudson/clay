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
    "",
    "  Connection:",
    "  /connect [host port [ssl]] Connect to server",
    "  /disconnect (or /dc)       Disconnect from server",
    "  /worlds                    Open world selector",
    "  /worlds <name>             Connect to or create world",
    "  /worlds -e [name]          Edit world settings",
    "  /worlds -l <name>          Connect without auto-login",
    "  /connections (or /l)       List connected worlds",
    "  /addworld <name> [host] [port] [-s]",
    "                             Add or update world (-s=SSL)",
    "  /keepalive                 Show keepalive settings",
    "",
    "  Communication:",
    "  /send [-W] [-w<world>] [-n] <text>",
    "                             Send text to world(s)",
    "                             -W=all worlds, -n=no newline",
    "  /notify <message>          Send notification to mobile",
    "",
    "  Lookup & Translation:",
    "  /dict <prefix> <word>      Look up word definition",
    "  /urban <prefix> <word>     Look up Urban Dictionary",
    "  /translate <lang> <prefix> <text>",
    "                             Translate text (or /tr)",
    "                             <lang> = code (es) or name (spanish)",
    "",
    "  Actions & Triggers:",
    "  /actions [world]           Open actions editor",
    "  /gag <pattern>             Gag lines matching pattern",
    "  /<action_name> [args]      Execute named action",
    "",
    "  Settings:",
    "  /setup                     Open global settings",
    "  /web                       Open web/WebSocket settings",
    "  /tag                       Toggle MUD tag display (F2)",
    "",
    "  Display:",
    "  /menu                      Open menu popup",
    "  /flush                     Clear output buffer",
    "  /dump                      Dump scrollback to file",
    "  /edit [file]               Open split-screen editor",
    "",
    "  System:",
    "  /help                      Show this help",
    "  /help tf                   Show TF commands help",
    "  /version                   Show version info",
    "  /reload                    Hot reload binary",
    "  /testmusic                 Test ANSI music playback",
    "  /quit                      Exit client",
    "",
    "  Security:",
    "  /ban                       Show banned hosts",
    "  /unban <host>              Remove host from ban list",
    "",
    "TF Commands:",
    "  For TinyFugue compatibility commands (triggers, macros,",
    "  variables, control flow), use:  /help tf  or  /tfhelp",
    "",
    "World Switching:",
    "  Up/Down                    Switch between active worlds",
    "  Shift+Up/Down              Switch between all worlds",
    "  Alt+W                      Switch to world with activity",
    "",
    "Input:",
    "  Left/Right, Ctrl+B/F       Move cursor",
    "  Ctrl+Up/Down               Move cursor up/down lines",
    "  Alt+Up/Down                Resize input area",
    "  Ctrl+U                     Clear input",
    "  Ctrl+W                     Delete word",
    "  Ctrl+P/N                   Command history",
    "  Ctrl+Q                     Spell suggestions",
    "  Ctrl+A, Home/End           Jump to start/end",
    "  Tab                        Command completion",
    "",
    "Output:",
    "  PageUp/PageDown            Scroll output",
    "  Tab                        Release one screenful (paused)",
    "  Alt+J                      Jump to end, release all",
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
            tab_buttons_only: false,
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
