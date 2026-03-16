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
    "Commands:               (/help <command> for details)",
    "",
    "  Connection:",
    "  /worlds                    Open world selector",
    "  /worlds <name>             Connect to or create world",
    "  /worlds -e [name]          Edit world settings",
    "  /worlds -l <name>          Connect without auto-login",
    "  /disconnect (or /dc)       Disconnect from server",
    "  /connections (or /l)       List connected worlds",
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
    "  /help [topic]              Show help (topic = command name)",
    "  /version                   Show version info",
    "  /reload                    Hot reload binary",
    "  /testmusic                 Test ANSI music playback",
    "  /quit                      Exit client",
    "",
    "  Security:",
    "  /ban                       Show banned hosts",
    "  /unban <host>              Remove host from ban list",
    "",
    "World Switching:",
    "  Up/Down                    Switch between active worlds",
    "  Shift+Up/Down              Switch between all worlds",
    "  Esc+B                      Previous world",
    "  Esc+F                      Next world",
    "  Esc+W                      Switch to world with activity",
    "",
    "Input:",
    "  Left/Right, Ctrl+B/F       Move cursor",
    "  Ctrl+Up/Down               Move cursor up/down lines",
    "  Alt+Up/Down                Resize input area",
    "  Ctrl+U                     Clear input",
    "  Ctrl+W                     Delete word before cursor",
    "  Ctrl+K                     Delete to end of line",
    "  Ctrl+D                     Delete character under cursor",
    "  Ctrl+T                     Transpose characters",
    "  Ctrl+V                     Insert next char literally",
    "  Ctrl+A/Home                Jump to start of line",
    "  Ctrl+E/End                 Jump to end of line",
    "  Esc+D                      Delete word forward",
    "  Esc+C                      Capitalize word forward",
    "  Esc+L                      Lowercase word forward",
    "  Esc+U                      Uppercase word forward",
    "  Esc+Space                  Collapse spaces to one",
    "  Esc+-                      Goto matching bracket",
    "  Esc+. or Esc+_             Insert last arg from history",
    "  Esc+Backspace              Delete word (punctuation)",
    "  Ctrl+P/N                   Command history",
    "  Esc+P                      Search history backward",
    "  Esc+N                      Search history forward",
    "  Ctrl+Q                     Spell suggestions",
    "  Tab                        Command completion",
    "",
    "Output:",
    "  PageUp/PageDown            Scroll output",
    "  Tab                        Release one screenful (paused)",
    "  Esc+J (lowercase)          Jump to end, release all",
    "  Esc+J (uppercase)          Selective flush (keep hilite)",
    "  Esc+H                      Half-page scroll/release",
    "  Ctrl+G                     Terminal bell",
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
    "",
    "TinyFugue Commands:     (/help <command> for details)",
    "",
    "  Variables:",
    "  /set [name [value]]        Set/list global variables",
    "  /unset name                Remove a variable",
    "  /let name value            Set a local variable",
    "  /setenv name               Export variable to env",
    "  /listvar [pattern]         List variables",
    "",
    "  Expressions:",
    "  /expr expression           Evaluate and display result",
    "  /test expression           Evaluate, set %?",
    "  /eval expression           Evaluate and execute as cmd",
    "",
    "  Control Flow:",
    "  /if (expr) cmd             Conditional execution",
    "  /while (expr) ... /done    While loop",
    "  /for var s e [step] /done  For loop",
    "  /break                     Exit loop",
    "",
    "  Macros/Triggers:",
    "  /def [opts] name = body    Define macro/trigger",
    "  /undef name                Remove macro",
    "  /list [pattern]            List macros",
    "  /purge [pattern]           Remove all macros",
    "",
    "  Hooks & Keys:",
    "  /bind key = command        Bind key to command",
    "  /unbind key                Remove key binding",
    "",
    "  Output:",
    "  /echo message              Display message locally",
    "  /beep                      Terminal bell",
    "  /quote [opts] text         Generate/send text from source",
    "  /tfgag pattern             Suppress matching lines (TF)",
    "  /ungag pattern             Remove gag",
    "  /recall [pattern]          Search output history",
    "",
    "  World Management:",
    "  /fg [name]                 Switch to or list worlds",
    "  /addworld [opts] name ...  Add/update world",
    "",
    "  File Operations:",
    "  /load [-q] filename        Load TF script",
    "  /require [-q] file         Load file if not loaded",
    "  /save filename             Save macros to file",
    "  /lcd path                  Change local directory",
    "",
    "  Process:",
    "  /repeat [opts] count cmd   Repeat command on timer",
    "  /ps                        List background processes",
    "  /kill id                   Kill background process",
    "",
    "  Misc:",
    "  /time                      Display current time",
    "  /sh command                Execute shell command",
    "  /input text                Insert text into input",
    "  /grab [world]              Grab last line into input",
    "  /trigger [pattern]         Manually trigger macros",
    "",
    "  Variable Substitution:",
    "  %{varname}                 Variable value",
    "  %1-%9, %*                  Positional params",
    "  %L, %R                     Left/right of match",
    "  %P0-%P9                    Regex capture groups",
    "  %%                         Literal percent sign",
    "",
    "Editors:             (open in browser via HTTP port)",
    "  /theme-editor              Customize GUI/web colors",
    "  /keybind-editor            Configure keyboard bindings",
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
        "edit" => vec![
            "/edit [file]               Open split-screen editor",
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
            "/dict <prefix> <word>",
            "",
            "Look up a word definition. The prefix is prepended",
            "to the output (e.g., 'say' to share with others).",
        ],
        "urban" => vec![
            "/urban <prefix> <word>",
            "",
            "Look up Urban Dictionary definition.",
        ],
        "translate" | "tr" => vec![
            "/translate <lang> <prefix> <text>  (or /tr)",
            "",
            "Translate text to the specified language.",
            "<lang> can be a code (es, fr) or name (spanish).",
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
