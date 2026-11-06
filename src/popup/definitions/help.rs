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

/// The first page of `/help` (no topic). The only copy: TF's `cmd_help` answers a bare
/// `/help` with it too, which is what an SSH console and the server's other routes show.
pub const HELP_LINES: &[&str] = &[
    "Getting Started:",
    "  /world               - World selector: add, edit, connect",
    "  /world <name>        - Connect/switch to a world",
    "  /dc                  - Disconnect from current world",
    "  /connections         - Show all connected worlds",
    "  /quit                - Exit Clay",
    "",
    "Keys:",
    "  PgUp / PgDn          - Scroll through output history",
    "  Esc-Left / Esc-Right - Switch connected worlds",
    "  Ctrl-P / Ctrl-N      - Command history",
    "  Tab                  - Page down; at the bottom, release held output",
    "  Shift-Tab            - Move to the other pane (/split)",
    "  Esc-e                - Open the emoji picker",
    "",
    "Basic Configuration:",
    "  /setup               - Settings (more mode, TTS, display, ...)",
    "  /web                 - Web server / remote access settings",
    "  /reach               - How other devices can reach this Clay",
    "  /actions             - Manage triggers and actions",
    "",
    "For more help:",
    "  /help commands       - List of commands",
    "  /help functions      - List of functions",
    "  /help <command>      - Help on a specific command (e.g. /help def)",
    "  /help keys           - Keyboard bindings (short list)",
    "  /help web            - Remote access, and the browser editors",
];

/// Topic-specific help content for Clay commands.
/// Returns None if the topic is not a Clay command (might be a TF command).
pub fn get_topic_help(topic: &str) -> Option<Vec<String>> {
    let lines: Vec<&str> = match topic {
        "worlds" | "world" => vec![
            "/worlds                    Open world selector popup",
            "/worlds <name>             Switch to a world, connecting it",
            "/worlds <host> <port>      Connect to a host/port (temp world)",
            "/worlds -e [name]          Edit a world (the current one if no",
            "                           name; a new name makes a new world)",
            "/worlds -l <name>          Connect without auto-login",
            "/worlds -b <name>          Connect in the background",
            "/worlds -x <host> <port>   Use SSL (a named world uses its own",
            "                           SSL setting)",
            "/world activity (or act)   Other worlds' new lines as they",
            "                           arrive (/help activity)",
            "",
            "/world is the same command (TinyFugue's name for it).",
            "-q, -n and -f are accepted, not distinct.",
            "",
            "The world selector shows all worlds with columns:",
            "  World name, Hostname, Port, User",
            "",
            "Controls in world selector:",
            "  Up/Down - Navigate    Enter - Connect",
            "  Tab - Cycle buttons   Esc - Close",
            "  Type to filter        F - Focus filter",
            "  A Add  E Edit  D Delete  O Connect  C Cancel",
        ],
        "disconnect" | "dc" => vec![
            "/disconnect (or /dc) [<world>|-ALL]   Disconnect a world",
            "",
            "With no argument, disconnects the current world. Given a world",
            "name, disconnects that world instead. Given -ALL (case-",
            "insensitive), disconnects every connected world.",
        ],
        "connections" | "l" | "listsockets" => vec![
            "/connections (or /l)       List connected worlds",
            "/listsockets [-s] [-S<field>] [<name>]  Same table, more options",
            "",
            "Shows a table with columns:",
            "  SSH     SSH: the connection uses TLS (PRX: through the",
            "          reload proxy)",
            "  World   * marks the current world",
            "  Unseen  Lines you haven't seen",
            "  Last    Since the last line received / sent",
            "  KA      Since the last keepalive / until the next",
            "  Buffer  Lines held for the world",
            "",
            "/listsockets -s prints one world name per line instead;",
            "-S<field> sorts by name/host/port/character/lines/idle.",
            "<name> lists only worlds whose name contains it.",
            "-n, -m and -T are accepted, not distinct.",
        ],
        "server" => vec![
            "/server <host[:port]>      Attach to a remote Clay server",
            "/server <host> <port>      Attach to a remote Clay server",
            "/server --close            Detach and become an independent master",
            "/server --cancel           Cancel a pending confirmation",
            "",
            "Relaunches this process as a remote client of the given Clay",
            "server (like starting with --console=host:port / --gui=host:port).",
            "From a master, this disconnects its own server and any connected",
            "worlds, so it asks for confirmation (repeat the command within",
            "15s to confirm). /server --close reverses this, detaching from",
            "whatever server you're attached to and starting a fresh,",
            "independent master. Only available from local console/GUI input.",
        ],
        "notify" => vec![
            "/notify <message>",
            "",
            "Send a notification, titled with the world's name: the",
            "Android app's, and the desktop's in both GUIs.",
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
        "setup" => vec![
            "/setup                     Open global settings",
            "",
            "Settings: more mode, spell check, temp convert, world",
            "switching, debug, input height, clear input on send,",
            "TLS proxy, editor side, ZWJ, ANSI music, status line,",
            "new indicator, text reveal, screen reader, TTS, archive,",
            "log input, wrap space, picture host, separator style.",
            "Import copies another Clay's settings (/help import).",
            "The ? button explains each one.",
        ],
        "web" => vec![
            "/web                       Web server settings",
            "/reach                     How other devices can reach this Clay",
            "",
            "One web server for the web page, the GUIs, the phone app",
            "and remote consoles: TLS for other devices, plain on this",
            "machine. /web sets its Port (or Disabled), Web Path,",
            "Remote Lines, Password, allow list, your own certificate",
            "and the Auth Key. Remote Access (also /reach) shows",
            "how other devices reach this Clay: addresses, firewall,",
            "router mapping.",
            "",
            "Browser editors, once /web has a port and a password:",
            "  http://localhost:<port>/theme-editor     GUI colours",
            "  http://localhost:<port>/action-editor    Actions",
            "  http://localhost:<port>/keybind-editor   Key bindings",
            "From another device: https://<host>:<port>/<web path>/...",
            "(/reach shows the port and addresses). The web page and",
            "the GUIs open all three from Settings.",
        ],
        "menu" => vec![
            "/menu                      Open the console's menu",
            "",
            "Lists every window and popup the console can open",
            "without anything more from you. Console only: the",
            "web and GUI interfaces have their \u{2630} menu.",
        ],
        "flush" => vec![
            "/flush                     Clear output buffer",
            "",
            "Clears the output buffer for the current world.",
        ],
        "dump" => vec![
            "/dump [-w world] [-n lines] Dump scrollback to file",
            "",
            "Writes Clay's state and every world's output buffer",
            "to dump.log in Clay's directory, on the machine Clay",
            "runs on. -w writes only that world; -n only the last",
            "that many lines of each world written. The space",
            "after -w and -n is optional.",
            "Example: /dump -w mymud -n 200",
        ],
        "mssp" => vec![
            "/mssp                      Show MSSP server status",
            "",
            "Shows the current world's MSSP (telnet option 70) status",
            "listing - name/value pairs like NAME, PLAYERS, UPTIME,",
            "CODEBASE - as reported by the server. Read-only: MSSP is",
            "server -> client only, so there is nothing to send.",
        ],
        "stats" => vec![
            "/stats                     Show the current world's status display",
            "/stats hide|show           This world's Status Panel switch",
            "",
            "Shows every stat derived from GMCP Char.* packages and MSDP",
            "variables - vitals like HP/MP first, then everything else.",
            "A paired current/maximum (e.g. hp/maxhp) shows as one gauge",
            "entry. Shown only when the MUD has sent this data; nothing is",
            "requested. The console shows a one-line summary above the",
            "input area (the GUIs a panel) while this world's switch and",
            "/setup's Status Line are both on. hide/show (also off/on)",
            "set this world's switch, as the panel's x does in a GUI.",
        ],
        "msdp" => vec![
            "/msdp <LIST|REPORT|UNREPORT|SEND> [target]",
            "                           Send an MSDP command",
            "",
            "Sends an MSDP (telnet option 69) command to the current",
            "world. LIST targets: COMMANDS, LISTS,",
            "CONFIGURABLE_VARIABLES, REPORTABLE_VARIABLES,",
            "SENDABLE_VARIABLES, REPORTED_VARIABLES.",
            "Refused (with a message) if MSDP was never negotiated.",
            "Example: /msdp REPORT HEALTH",
        ],
        "note" | "notes" => vec![
            "/note [file]               Open split-screen editor",
            "/note -l                   List the worlds' notes",
            "",
            "Opens a notes editor. With no args, opens per-world",
            "notes. With a filename, opens that file for editing.",
            "Ctrl+S saves and closes, Esc closes without saving,",
            "Ctrl+Space moves between the editor and the input.",
            "Every interface has all three. A file is always on",
            "the main instance's machine: a remote console, the",
            "web page, the GUI and the app edit the server's.",
        ],
        "help" | "tfhelp" => vec![
            "/help [topic]              Show help",
            "",
            "Without a topic, shows the first help page.",
            "With a topic, shows help for that specific command.",
            "/tfhelp is the same command.",
            "",
            "Works for both Clay and TF commands:",
            "  /help worlds     - World management commands",
            "  /help def        - TF macro definition",
            "  /help functions  - TF expression functions",
            "  /help send       - Send command details",
            "",
            "A topic Clay doesn't cover comes from TinyFugue's help",
            "file when tf is installed (/help intro, /help kecho).",
            "Those topics are case-sensitive, as in TF.",
        ],
        "version" => vec![
            "/version                   Show version info",
            "",
            "Attached to another Clay (a GUI with --gui=host, the",
            "--console=host or SSH console, the Android app on a",
            "server), it shows this program's build and the server's:",
            "  Clay: Local  v1.7.12 (build ...) [windows/x86_64]",
            "        Remote v1.7.11 (build ...) [linux/x86_64]",
        ],
        "reach" => vec![
            "/reach                     How other devices can reach this Clay",
            "",
            "Shows this machine's LAN/VPN/public addresses, the Windows",
            "Firewall and router (UPnP) status, and exactly what to type",
            "on another device. The same panel is at /web -> Remote Access.",
            "",
            "/reach --refresh (-r)      Re-check firewall and router mapping",
            "/reach --lookup (-l)       Look up the public IP (asks",
            "                           checkip.amazonaws.com; never automatic)",
        ],
        "import" => vec![
            "/import [host[:port]]      Import settings from another Clay",
            "",
            "Copies another Clay's settings into this one, asking for",
            "its password or auth key (the host filled in if given).",
            "In the web page and the GUIs it is also the Import button",
            "on Settings' General tab. The main instance's own: a",
            "remote console can't.",
        ],
        "map" => crate::map::app::MAP_HELP.lines().chain([
            "",
            "Built from the rooms the game reports over GMCP. While",
            "TinyFugue's map.tf is loaded, /@map reaches Clay's.",
        ]).collect(),
        "reload" => vec![
            "/reload                    Hot reload binary",
            "/reload --console          ... coming back as the console",
            "/reload --gui              ... coming back as the GUI",
            "",
            "Saves state, exec()s new binary, restores state.",
            "TCP connections are preserved (TLS needs proxy).",
            "A reload stays in the interface it was in; when",
            "that can't run there (no display, no terminal),",
            "it comes back in the other and says so.",
            "--console/--gui switch it, if the other can run:",
            "a GUI needs a display, the console a terminal.",
            "An attached GUI or console switches itself and",
            "stays attached to the same server.",
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
            "Plays a short tune (Super Mario Bros) to test audio.",
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
        "translate" => vec![
            "/translate <lang> <text>",
            "",
            "Translate text to the specified language.",
            "<lang> can be a code (es, fr) or name (spanish).",
            "Result is placed in the input buffer with cursor at",
            "start — type a prefix then press Enter to send.",
            "(/tr is TinyFugue's character translation: /help tr.)",
        ],
        "url" => vec![
            "/url <url>",
            "",
            "Shorten a URL, trying several services until one",
            "works. Result is placed in the input buffer with",
            "cursor at start — type a prefix (e.g. 'say') then",
            "press Enter to send.",
        ],
        "remote" => vec![
            "/remote                    List connected clients",
            "/remote --kill <id>        Disconnect a client",
            "/remote --pause <id>       Toggle pause on a client",
            "/remote --pause console    Toggle pause on the console",
            "/pause                     Pause the other interfaces",
            "",
            "Lists all connected WebSocket clients with their ID,",
            "address, client type, ping liveness, and pause status.",
            "Use --kill <id> to forcibly disconnect a client.",
            "Use --pause <id> to pause/resume a client. While paused,",
            "activity on that session's world is visible to other",
            "sessions. The client resumes as soon as it is used.",
            "--pause console does the same for the console, from any",
            "interface: activity on the world it shows becomes new",
            "activity for everyone else. Any key pressed at the",
            "console resumes it.",
        ],
        "pause" => vec![
            "/pause                     Pause every other interface",
            "",
            "/remote --pause on the console and on every client but",
            "the one you typed it in (its other windows and tabs are",
            "left alone). A paused interface stops counting as",
            "watching its world, so activity there shows as new",
            "where you are. Each one resumes as soon as it is used.",
            "/remote shows which are paused.",
        ],
        "font" => vec![
            "/font                      Font settings (web/GUI)",
            "",
            "Opens Settings' Font tab in the web page and the GUIs:",
            "the font, its size per device, weight, line height and",
            "spacing. Each program keeps its own fonts on each OS.",
            "The console's font is the terminal's.",
        ],
        "update" => vec![
            "/update [-f]               Download and install update",
            "",
            "Installs the latest release from GitHub and reloads,",
            "keeping connections. -f (--force): even if not newer.",
            "In a GUI or console attached to another Clay, the server",
            "is updated first, then this program. clay --upgrade",
            "installs it from a shell.",
        ],
        "activity" | "act" => vec![
            "/world activity  (or /world act)",
            "/split activity  (or /split act)",
            "",
            "The Activity world shows the new lines of every world you",
            "aren't looking at, as they arrive, each after the name of",
            "the world it came from, on a block of the New Indicator's",
            "colour (yellow when it has none). It never pauses for more,",
            "and nothing it shows is logged, fires a trigger or counts",
            "as activity: the lines did all that in their own worlds.",
            "",
            "Each interface has its own: it holds only what arrives",
            "while it is on screen there, starting empty each time you",
            "go to it, and leaves out any world that interface shows",
            "itself (the other pane, another window). Gagged and blank",
            "lines and Clay's own messages aren't shown. The switch keys",
            "and Esc-w never go to it, and no world list shows it;",
            "a world of your own named Activity takes its place.",
        ],
        "split" | "unsplit" => vec![
            "/split                     Split the output: the next world above",
            "/split <world>             Split with <world> above",
            "/split act                 Other worlds' new lines above",
            "                           (/help activity)",
            "/split -n <lines> [world]  The top pane <lines> high (split,",
            "                           -n alone just resizes it)",
            "/unsplit                   Close the pane without the focus",
            "",
            "Two worlds at once, one pane above the other. The world",
            "you are in stays below and keeps the focus: what you type,",
            "commands and keys go to the focused pane. Shift-Tab moves",
            "the focus (in the GUI, so does a click on a pane). Its bar",
            "keeps the bar colour; the other pane's turns grey.",
            "",
            "The switch-world keys skip the world in the other pane;",
            "/world <name> may show one world in both. Already split,",
            "/split <world> puts it in the pane without the focus.",
            "Nothing connects or disconnects; the split survives",
            "/reload.",
            "",
            "Each pane keeps one line at least. The size, set by -n or",
            "(in the GUI and the web page) by dragging the bar between",
            "the panes, is this program's own, and the next /split",
            "opens at it again.",
        ],
        "window" => vec![
            "/window [world]            Open another window",
            "/window --grep <pattern> [-w <world>] [--regexp]",
            "                           Open a grep/filter window",
            "",
            "Opens another window of the web page or GUI, kept to",
            "<world> if one is named. The --grep form opens a half-",
            "height window showing only the lines of the current",
            "world (or <world>) that match <pattern>: a glob,",
            "or a regex with --regexp. Not in a console.",
        ],
        "say" => vec![
            "/say <text>                Speak text aloud (TTS)",
            "",
            "Speaks <text> in the interface it was typed in, with that",
            "interface's own TTS (set in its Settings; the console's",
            "in /setup): Local (espeak, say, PowerShell), Kokoro (the",
            "neural voice from /tts install) or Edge (cloud). Typed at",
            "the console or run by an action, every interface speaks",
            "it, each with its own.",
        ],
        "tts" => vec![
            "/tts                       Text-to-speech status",
            "/tts install               Fetch the Kokoro voice files",
            "/tts remove                Delete them again",
            "/tts voices                List Kokoro's 54 voices",
            "/tts voice <name>          Pick the Kokoro voice",
            "/tts test                  Speak a test line",
            "",
            "Kokoro is an offline neural voice. /tts install downloads",
            "its runtime and the model build this CPU runs fastest",
            "(160 to 340 MB) into ~/.clay/tts; then set TTS to Kokoro",
            "in this interface's Settings (the console's: /setup).",
            "Until the files are there, Kokoro mode speaks with the",
            "Local engine. A static build (the musl console) cannot",
            "load it; use the GUI build. A GUI attached to another",
            "Clay uses its own machine's files: /tts install there",
            "fetches them onto that machine (clay --tts-install too).",
        ],
        _ => return None,
    };
    Some(lines.into_iter().map(|s| s.to_string()).collect())
}

/// Create the help popup definition (main help, no topic)
///
/// Titled "Help for Clay vX.Y.Z" so the build is visible without running `/version` — matching
/// the web/GUI/Android `#help-modal`, which is the only place those clients surface their own
/// version at all (see `openHelpPopup` in app.js). The per-topic popup below deliberately
/// keeps the plain "Help" title: it's contextual help for one setting, not the app's own help,
/// and its content already names its subject.
pub fn create_help_popup() -> PopupDefinition {
    PopupDefinition::new(PopupId("help"), format!("Help for Clay v{}", crate::VERSION))
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
        // The title carries the running build so a user can identify it without /version.
        assert_eq!(state.definition.title, format!("Help for Clay v{}", crate::VERSION));
        assert!(state.definition.title.contains(crate::VERSION),
            "the version must be the real one, not a hardcoded copy that can go stale");
        assert!(!state.definition.buttons.is_empty());
    }

    #[test]
    fn test_topic_help() {
        assert!(get_topic_help("worlds").is_some());
        assert!(get_topic_help("nonexistent").is_none());
    }

    /// The first page names each command once (`/setup` used to be on it twice) and
    /// leaves the browser editors to `/help web`: they need `/web` set up first.
    #[test]
    fn the_first_page_names_each_command_once_and_no_editor() {
        let mut seen = std::collections::HashSet::new();
        for line in HELP_LINES.iter().filter(|l| l.trim_start().starts_with('/')) {
            // What is typed: "/world" and "/world <name>" are two entries.
            let typed = line.split(" - ").next().unwrap_or(line).trim();
            assert!(seen.insert(typed), "listed twice on the first page: {line:?}");
        }
        assert!(!HELP_LINES.iter().any(|l| l.contains("-editor")), "the editors are /help web's");
        assert!(get_topic_help("web").unwrap().iter().any(|l| l.contains("/keybind-editor")));
    }

    /// One first page: TF's bare `/help`, which the SSH console and the server's other
    /// routes show, is this one. Two copies drifted apart.
    #[test]
    fn a_bare_help_is_this_page_everywhere() {
        assert_eq!(crate::tf::parser::help_text(&crate::tf::TfEngine::new(), ""), HELP_LINES.join("\n"));
    }

    /// A topic lives in one file. The console asks this file first, every other
    /// interface asks TF's `cmd_help` first, so a topic both defined showed different
    /// text depending on where `/help <topic>` was typed (a GUI's `/help version` said
    /// "Display Clay version information.").
    #[test]
    fn every_clay_topic_is_answered_the_same_by_tf() {
        let src = include_str!("help.rs");
        let body = &src[src.find("pub fn get_topic_help").unwrap()..];
        let body = &body[..body.find("        _ => return None,").unwrap()];
        let mut names = Vec::new();
        for line in body.lines() {
            let Some(arms) = line.strip_prefix("        \"").and_then(|l| l.split(" =>").next()) else { continue };
            names.extend(arms.split('|').map(|a| a.trim().trim_matches('"')));
        }
        assert!(names.len() > 40, "{names:?}");
        let engine = crate::tf::TfEngine::new();
        for name in names {
            let lines = get_topic_help(name).unwrap_or_else(|| panic!("{name:?} has an arm but no help"));
            assert_eq!(crate::tf::parser::help_text(&engine, name), lines.join("\n"),
                "/help {name}: TF's cmd_help has its own text - keep one");
        }
    }
}
