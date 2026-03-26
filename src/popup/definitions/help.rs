//! Help popup definition
//!
//! Displays help content with keyboard shortcuts and commands.
//! Supports topic-specific help via /help <topic>.

use crate::popup::{
    Button, ButtonId, Field, FieldId, FieldKind, PopupDefinition, PopupId, PopupLayout,
};

// Field IDs
pub const HELP_FIELD_CONTENT: FieldId = FieldId(1);

// Button IDs
pub const HELP_BTN_OK: ButtonId = ButtonId(1);

/// Main help content lines (no topic specified)
pub const HELP_LINES: &[&str] = &[
    "Getting Started:",
    "  /setup               - Open settings (server, colors, etc.)",
    "  /world               - Setup connection(s) to a world(s)",
    "  /world <name>        - Connect/switch to a world",
    "  /dc                  - Disconnect from current world",
    "  /connections         - Show all connected worlds",
    "  /quit                - Exit Clay",
    "",
    "Keys:",
    "  PgUp / PgDn          - Scroll through output history",
    "  Ctrl-Up or Down      - Switch Worlds",
    "  Tab                  - Release world output when paused.",
    "",
    "Basic Configuration:",
    "  /setup               - General settings popup",
    "  /web                 - Web interface / remote access settings",
    "  /actions             - Manage triggers and actions",
    "  /keybinds            - Open keybinding editor (browser)",
    "",
    "For more help:",
    "  /help commands       - List of commands",
    "  /help functions      - List of functions",
    "  /help <command>      - Help on a specific command (e.g. /help def)",
    "  /help keys           - All keyboard bindings",
    "  /help web            - Websocket help (remote interfaces)",
];

/// Topic-specific help content for Clay commands.
/// Returns None if the topic is not a Clay command (might be a TF command).
pub fn get_topic_help(topic: &str) -> Option<Vec<String>> {
    let lines: Vec<&str> = match topic {
        "worlds" | "world" => vec![
            "/worlds                    Open world selector popup",
            "/worlds <name>             Connect to or create world",
            "/worlds -e [name]          Edit world settings",
            "/worlds -l <name>          Connect without auto-login",
            "",
            "The world selector shows all worlds with columns:",
            "  World name, Hostname, Port, User",
            "",
            "Controls in world selector:",
            "  Up/Down - Navigate    Enter - Connect",
            "  Tab - Cycle buttons   Esc - Close",
            "  Type to filter        / - Focus filter",
        ],
        "disconnect" | "dc" => vec![
            "/disconnect (or /dc)       Disconnect current world",
            "",
            "Closes the connection and log file for the current world.",
        ],
        "connections" | "l" => vec![
            "/connections (or /l)       List connected worlds",
            "",
            "Shows a table with columns:",
            "  World  Unseen  LastSend  LastRecv  LastNOP  NextNOP",
            "  * marks the current world",
        ],
        "send" => vec![
            "/send [-W] [-w<world>] [-n] <text>",
            "",
            "Send text to a world.",
            "  -w<world>  Send to specified world (by name)",
            "  -W         Send to all connected worlds",
            "  -n         Send without end-of-line marker",
            "  No flags: Send to current world",
        ],
        "notify" => vec![
            "/notify <message>",
            "",
            "Send a push notification to the Android app.",
            "Can be used in action commands: /notify Page from $1",
        ],
        "actions" => vec![
            "/actions [world]           Open actions editor",
            "",
            "Actions match incoming MUD output and execute commands.",
            "  Pattern: regex or wildcard (empty = manual-only)",
            "  Command: semicolon-separated, $1-$9 for captures",
            "  /gag in commands hides matched line",
            "  Enable 'Startup' to run on Clay start/reload",
        ],
        "gag" => vec![
            "/gag <pattern>             Gag lines matching pattern",
            "",
            "Creates a gag action that hides lines matching the",
            "regex pattern. Gagged lines are still visible with F2.",
        ],
        "setup" => vec![
            "/setup                     Open global settings",
            "",
            "Settings: more mode, spell check, temp convert,",
            "world switching, show tags, input height, themes,",
            "mouse, ZWJ, ANSI music, TLS proxy",
        ],
        "web" => vec![
            "/web                       Open web/WebSocket settings",
            "",
            "Configure WebSocket (ws/wss), HTTP/HTTPS servers,",
            "TLS certificates, passwords, and allow lists.",
        ],
        "menu" => vec![
            "/menu                      Open menu popup",
            "",
            "Provides access to common commands and popups.",
        ],
        "flush" => vec![
            "/flush                     Clear output buffer",
            "",
            "Clears the output buffer for the current world.",
        ],
        "dump" => vec![
            "/dump                      Dump scrollback to file",
            "",
            "Writes the current world's output buffer to a file.",
        ],
        "note" | "notes" => vec![
            "/note [file]               Open split-screen editor",
            "",
            "Opens a notes editor. With no args, opens per-world",
            "notes. With a filename, opens that file for editing.",
        ],
        "help" => vec![
            "/help [topic]              Show help",
            "",
            "Without a topic, shows the full help popup.",
            "With a topic, shows help for that specific command.",
            "",
            "Works for both Clay and TF commands:",
            "  /help worlds     - World management commands",
            "  /help def        - TF macro definition",
            "  /help functions  - TF expression functions",
            "  /help send       - Send command details",
        ],
        "version" => vec![
            "/version                   Show version info",
        ],
        "reload" => vec![
            "/reload                    Hot reload binary",
            "",
            "Saves state, exec()s new binary, restores state.",
            "TCP connections are preserved (TLS needs proxy).",
            "Also: Ctrl+R or kill -USR1 $(pgrep clay)",
        ],
        "quit" => vec![
            "/quit                      Exit the client",
        ],
        "ban" => vec![
            "/ban                       Show banned hosts",
            "",
            "Lists all hosts currently banned from connecting",
            "to the WebSocket server.",
        ],
        "unban" => vec![
            "/unban <host>              Remove host from ban list",
        ],
        "testmusic" => vec![
            "/testmusic                 Play test ANSI music",
            "",
            "Plays C-D-E-F-G to verify audio is working.",
        ],
        "tag" | "tags" => vec![
            "/tag                       Toggle MUD tag display (F2)",
            "",
            "Shows/hides MUD tags and timestamps on lines.",
            "Tags: [name:] or [name(content)] at line start.",
        ],
        "dict" => vec![
            "/dict <word>",
            "",
            "Look up a word in the dictionary. Result is placed in",
            "the input buffer with cursor at start — type a prefix",
            "(e.g. 'say') then press Enter to send.",
        ],
        "urban" => vec![
            "/urban <word>",
            "",
            "Look up Urban Dictionary definition. Result is placed",
            "in the input buffer with cursor at start — type a prefix",
            "(e.g. 'say') then press Enter to send.",
        ],
        "translate" | "tr" => vec![
            "/translate <lang> <text>  (or /tr)",
            "",
            "Translate text to the specified language.",
            "<lang> can be a code (es, fr) or name (spanish).",
            "Result is placed in the input buffer with cursor at",
            "start — type a prefix then press Enter to send.",
        ],
        "url" => vec![
            "/url <url>",
            "",
            "Shorten a URL using is.gd. Result is placed in the",
            "input buffer with cursor at start — type a prefix",
            "(e.g. 'say') then press Enter to send.",
        ],
        "remote" => vec![
            "/remote                    List connected clients",
            "/remote --kill <id>        Disconnect a client",
            "",
            "Lists all connected WebSocket clients with their ID,",
            "address, client type, and ping liveness status.",
            "Use --kill <id> to forcibly disconnect a client.",
        ],
        "font" => vec![
            "/font                      Font settings (web/GUI)",
            "",
            "Adjusts font size in the web and GUI interfaces.",
        ],
        "update" => vec![
            "/update [-f]               Download and install update",
            "",
            "Downloads latest release from GitHub.",
            "  -f  Force update even if version matches",
        ],
        "addworld" => vec![
            "/addworld [-Lq] name host port",
            "",
            "Create a new world (TF-compatible).",
            "Also: /addworld name [char pass] host port",
            "  -x  Use SSL/TLS for connection",
        ],
        _ => return None,
    };
    Some(lines.into_iter().map(|s| s.to_string()).collect())
}

/// Create the help popup definition (main help, no topic)
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
            anchor_bottom_left: false,
            anchor_x: 0,
        })
}

/// Create a help popup for a specific topic
pub fn create_topic_help_popup(lines: Vec<String>) -> PopupDefinition {
    PopupDefinition::new(PopupId("help"), "Help")
        .with_field(Field::new(
            HELP_FIELD_CONTENT,
            "",
            FieldKind::scrollable_content(lines, 100),
        ))
        .with_button(Button::new(HELP_BTN_OK, "Ok").primary().with_shortcut('O'))
        .with_layout(PopupLayout {
            label_width: 0,
            min_width: 1000,
            max_width_percent: 90,
            center_horizontal: true,
            center_vertical: false,
            modal: true,
            buttons_right_align: false,
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

    #[test]
    fn test_help_popup_creation() {
        let def = create_help_popup();
        let state = PopupState::new(def);

        assert_eq!(state.definition.id, PopupId("help"));
        assert_eq!(state.definition.title, "Help");
        assert!(!state.definition.buttons.is_empty());
    }

    #[test]
    fn test_topic_help() {
        assert!(get_topic_help("worlds").is_some());
        assert!(get_topic_help("nonexistent").is_none());
    }
}
