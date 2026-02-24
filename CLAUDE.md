# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Debug Output

**IMPORTANT: Never write debug output to stdout or stderr (no `println!`, `eprintln!`, `dbg!`).**

Debug output interferes with the TUI and corrupts the terminal display. Instead:
- Write debug logs to a file (e.g., `clay.debug.log`, `clay.tf.debug.log`)
- Or display messages in the output area using `add_tf_output()` or similar methods

## Build Commands

**IMPORTANT: Always use the musl debug build. Never use release builds.**

```bash
# Default build command (always use this)
cargo build --target x86_64-unknown-linux-musl --no-default-features --features rustls-backend

# Output: target/x86_64-unknown-linux-musl/debug/clay
```

Other commands:
```bash
cargo build --features remote-gui    # Build with remote GUI client (requires X11/Wayland)
cargo run                            # Run the client
cargo run -- --remote=host:port      # Run as remote GUI client
cargo test                           # Run all tests
cargo test test_name                 # Run a single test
cargo clippy                         # Lint
cargo fmt                            # Format code
cargo fmt -- --check                 # Check formatting without changes
```

### Why musl

The musl build produces a portable binary that works on any Linux x86_64 system regardless of glibc version.

```bash
# Install musl target (one-time setup)
rustup target add x86_64-unknown-linux-musl
```

**Why musl instead of glibc static linking:**
- glibc static builds cause SIGFPE crashes during DNS resolution (`getaddrinfo`)
- glibc's NSS (Name Service Switch) requires dynamic loading even in static builds
- musl handles DNS resolution properly in fully static binaries

**Why rustls instead of native-tls:**
- native-tls requires OpenSSL, which needs cross-compilation setup for musl
- rustls is pure Rust and works seamlessly with musl builds

### Building for Termux (Android)

Clay compiles and runs on Termux (Android terminal emulator):

```bash
# On Termux, install rust first
pkg install rust

# Build without GUI features (console-only)
cargo build --no-default-features --features rustls-backend
```

**Building with Remote GUI on Termux (requires Termux:X11):**

```bash
# Install X11 dependencies
pkg install x11-repo
pkg install libx11 libxcursor libxrandr libxi libxfixes

# Apply patches to winit/glutin for Android X11 support (one-time setup)
# This creates stubs, runs cargo fetch, then applies patches automatically
./patches/apply-patches.sh

# Build with GUI
cargo build --no-default-features --features rustls-backend,remote-gui

# Run GUI client (from within Termux:X11 desktop)
./target/debug/clay --gui=hostname:port
```

The patches modify winit, glutin, and glutin-winit to use the X11/EGL backend on Android instead of the Android-native windowing backend. Stub `Cargo.toml` and `src/lib.rs` files are checked into git so that `[patch.crates-io]` resolves on fresh clones without running `apply-patches.sh`. Only Termux needs to run the script to replace stubs with real patched sources for `--features remote-gui` builds.

**Limitations on Termux:**
- Hot reload (`/reload`, `Ctrl+R`, `SIGUSR1`) is not available - exec() and signals are limited on Android
- TLS proxy is not available - TLS connections cannot be preserved across restarts
- Process suspension (`Ctrl+Z`) is not available

**What works on Termux:**
- Core MUD client (connect, send, receive)
- Remote GUI client (with Termux:X11 and patches applied)
- Multiple worlds
- TLS connections (direct, not via proxy)
- More-mode pausing
- Actions/triggers
- Command history
- All TUI features
- Settings persistence

### Building for macOS

Clay compiles and runs on macOS (Intel and Apple Silicon):

```bash
# Install Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Build without GUI (terminal client only)
cargo build --no-default-features --features rustls-backend

# Build with GUI client
cargo build --features remote-gui

# Build with GUI and audio support
cargo build --features remote-gui-audio

# Output: target/debug/clay
```

**Building a Universal Binary (Intel + Apple Silicon):**

```bash
# Install both targets (one-time setup)
rustup target add x86_64-apple-darwin
rustup target add aarch64-apple-darwin

# Build for both architectures with GUI and audio
cargo build --release --target x86_64-apple-darwin --features remote-gui-audio
cargo build --release --target aarch64-apple-darwin --features remote-gui-audio

# Combine into universal binary
lipo -create \
    target/x86_64-apple-darwin/release/clay \
    target/aarch64-apple-darwin/release/clay \
    -output clay-macos-universal

# Verify (should show "universal binary with 2 architectures")
file clay-macos-universal
```

**Notes:**
- No musl target needed on macOS (use native libc)
- macOS uses native Cocoa windowing (no X11/Wayland required)
- Audio uses CoreAudio automatically
- Works on both Intel (x86_64) and Apple Silicon (aarch64)
- Universal binaries run natively on both architectures without Rosetta 2

**What works on macOS:**
- All core features (connect, send, receive, multiple worlds)
- TLS/SSL connections
- Hot reload (`/reload`, `Ctrl+R`, `SIGUSR1`)
- TLS proxy for preserving connections across reload
- Remote GUI client (native Cocoa window)
- ANSI music playback (with `remote-gui-audio` feature)
- WebSocket/HTTP/HTTPS servers
- Spell checking (`/usr/share/dict/words` exists on macOS)
- All TUI features

## Architecture

Clay is a terminal-based MUD (Multi-User Dungeon) client built with ratatui/crossterm for TUI and tokio for async networking. Supports multiple simultaneous world connections with SSL/TLS support.

### Key Structs

- **App**: Central state - worlds list, current world index, input area, spell checker, settings, settings popup
- **World**: Per-connection state - output buffer, scroll position, connection, unseen lines, pause state, world settings, log handle, prompt (from telnet GA)
- **WorldSettings**: Per-world connection settings - hostname, port, user, password, use_ssl, log_file, encoding, auto_connect_type, keep_alive_type, keep_alive_cmd
- **InputArea**: Input buffer with viewport scrolling, command history, cursor position
- **SpellChecker**: System dictionary-based spell checking with Levenshtein-distance suggestions (uses /usr/share/dict/words)
- **Settings**: Global preferences (more_mode_enabled, spell_check_enabled, world_switch_mode, websocket_enabled, websocket_port, websocket_password, https_enabled, https_port)
- **WebSocketServer**: Embedded WebSocket server for remote GUI clients
- **PopupManager**: Unified popup system for all modal dialogs (world editor, setup, web settings, help, world selector, actions, confirmations)
- **FilterPopup**: Modal filter popup for searching/filtering output text (F4)
- **Encoding**: Character encoding enum (Utf8, Latin1, Fansi) with decode method
- **StreamReader/StreamWriter**: Wrapper enums supporting both plain TCP and TLS streams
- **OutputLine**: Output line with timestamp and `from_server` flag (true = MUD server data, false = client-generated)

### Screen Layout

```
Output Area                    <- Takes up rest of screen, ANSI color support
Shows one world at a time
(no border)
 world [activity] [history]____time  <- Separator bar (underscores)
input line 1                          <- Input area (no border)
input line 2
input line 3
```

On startup, each world displays a colorful ASCII art splash screen with the tagline "A 90dies MUD client written today" and quick command hints.

### Separator Bar Components

- Status indicator (leftmost, 10 chars, black text on red background):
  - `More XXXX` - pending lines when paused (priority 1)
  - `Hist XXXX` - lines scrolled back in history (priority 2)
  - Underscores when neither active (dark gray)
  - Numbers are right-justified: 9999 → "9999", 10000 → " 10K", 999000 → "999K", 1000000+ → "Alot"
- Connection indicator and world name (only shown when connected):
  - Green ball (🟢) followed by world name (bold white)
  - When disconnected, this area is filled with underscores instead
- Activity indicator at position 24: `(Activity: X)` or `(Act X)` on narrow screens - count of worlds with unseen output (yellow, hidden if 0)
- Underscore padding fills remaining space (dark gray)
- Current time HH:MM format (right, cyan, no AM/PM)

### Scrollback Buffer

- Each world has unlimited scrollback (`Vec<String>` grows with available memory)
- `scroll_offset` tracks current position in buffer (logical line index)
- `is_at_bottom()` checks if viewing latest output
- `lines_from_bottom()` calculates history depth for indicator
- PageUp/PageDown scroll by visual lines (accounting for line wrapping)
- Manual line wrapping preserves ANSI color codes across wrapped lines

### Word Wrapping

All interfaces (console, web, GUI) use consistent word wrapping for long words:

- Words longer than 15 characters can break at specific characters: `[ ] ( ) , \ / - & = ?`
- Period (`.`) is excluded to avoid breaking filenames (e.g., `image.png`) and domain names
- Console: `wrap_ansi_line` tracks break opportunities and uses them when wrapping long words
- Web: `insertWordBreaks()` inserts zero-width spaces after break characters
- GUI: `insert_word_breaks()` inserts zero-width spaces, skipping ANSI escape sequences

This produces cleaner URL wrapping:
```
https://example.com/path/to/file.png?key=value&other=data
```
Breaks at `/`, `?`, `&`, `=` instead of at periods in filenames.

### Character Encoding

Encoding is configurable per-world in the world settings popup.

- **Utf8**: Standard UTF-8 (default) - `String::from_utf8_lossy`
- **Latin1**: ISO-8859-1 - direct byte-to-Unicode mapping
- **Fansi**: CP437-like for MUD graphics - box drawing chars, block elements
- Raw bytes passed via `AppEvent::ServerData(world_idx, Vec<u8>)`, decoded in main loop
- Control characters filtered during decode (keeps tab, newline, escape for ANSI)

**Colored Square Emoji:**
- Colored square emoji (🟥🟧🟨🟩🟦🟪🟫⬛⬜) are rendered with proper colors
- Console: Replaced with ANSI true-color block characters (██) since terminal emoji fonts ignore foreground colors
- Remote GUI: Rendered as colored rectangles using egui
- Web: Native emoji rendering (browser handles colors)
- Implementation: `colorize_square_emojis()` in encoding.rs, `has_colored_squares()` in GUI

**Display Width Handling:**
- Input area uses unicode display width for cursor positioning and line breaking
- Zero-width characters (U+200B, etc.) are handled correctly - they take no visual space
- Wide characters (CJK, emoji) are handled correctly - they take 2 columns
- Helper functions in `src/input.rs`: `display_width()`, `display_width_chars()`, `chars_for_display_width()`
- Ensures cursor position matches visual text position even with mixed-width characters
- First line has reduced capacity due to prompt; subsequent lines use full terminal width
- Viewport scrolling correctly accounts for prompt when calculating character positions

### Multi-World System

- Each world has independent: output buffer, scroll position, connection, unseen line count
- `unseen_lines` increments when output arrives for a non-current world
- `first_unseen_at` tracks when unseen output first arrived (for "Unseen First" world switching)
- Lines are marked as seen when the world's output is rendered to the console
- Switching worlds triggers a redraw, which clears the unseen count for the newly viewed world
- WebSocket clients receive `UnseenCleared` broadcast when a world is marked as seen

### More-Style Pausing

- Enabled via settings popup (default: on)
- Auto-triggers after `output_height - 2` lines of new output since last user input
- `lines_since_pause` counter tracks lines received; reset to 0 when user sends any command
- When pause triggers, scrolls to bottom first to show buffered output, then pauses
- Also triggered when user scrolls up with PageUp
- Incoming lines queue to `pending_lines` instead of `output_lines`
- Tab releases one screenful minus 2 lines of pending lines
- Escape+j releases all pending lines (jump to end)
- Enter sends command but does NOT release pending lines (only Tab or PageDown releases pending)
- Scrolling to bottom with PageDown also unpauses and releases all pending lines

### SSL/TLS Support

- Uses `tokio-native-tls` for TLS connections
- Enable via "Use SSL" toggle in world settings
- `StreamReader`/`StreamWriter` enums wrap both plain TCP and TLS streams
- Implements `AsyncRead`/`AsyncWrite` traits for unified handling

### TLS Proxy (Optional)

When enabled, TLS connections use a proxy process to preserve connections across hot reloads.

**Problem:** TLS encryption state (session keys, IVs, sequence numbers) exists only in process memory. When `exec()` replaces the process during hot reload, this state is lost even though the underlying TCP socket fd can survive.

**Solution:** A forked child process handles TLS, communicating with the main process via Unix socket:

```
MUD Server (TLS) <--TLS--> TLS Proxy (child) <--Unix Socket--> Main Client
```

**Configuration:**
- Toggle "TLS Proxy" in Global Settings (`/setup`) - default: disabled
- When enabled, new TLS connections spawn a proxy process
- Proxy process survives `exec()` and reconnects after reload

**Behavior:**
- Proxy spawns on TLS connect, dies on disconnect or main client exit
- Socket path: `/tmp/clay-tls-<pid>-<world_name>.sock`
- Proxy PID and socket path saved/restored during reload
- Health monitoring detects proxy death and marks world disconnected
- Falls back to direct TLS if proxy spawn fails

**Implementation:**
- `spawn_tls_proxy()` - Forks child process
- `run_tls_proxy()` - Child main loop (TLS ↔ Unix socket relay)
- `StreamReader::Proxy` / `StreamWriter::Proxy` - Unix socket variants
- Proxy state saved in reload: `proxy_pid`, `proxy_socket_path`

### File Logging

- Configure log file path in world settings popup
- Log file opened on connect (append mode), closed on disconnect
- All received output is written to log file with each line
- Uses `Arc<Mutex<File>>` for thread-safe access

### Auto-Login

Configure user, password, and auto login type in world settings. Three modes are available:

- **Connect** (default): Sends `connect <user> <password>` command 500ms after connection
- **Prompt**: Sends username on first telnet GA prompt, password on second prompt
- **MOO_prompt**: Like Prompt, but also sends username again on third prompt (for MOO-style login)

Credentials stored per-world. Auto-login only triggers if both username AND password are configured.
Prompts that are auto-answered are immediately cleared and not displayed in the input area.

### Telnet Protocol Support

- Automatic telnet negotiation handling
- Strips telnet sequences from displayed output
- Detects telnet mode when IAC sequences are received
- Configurable keepalive sent every 5 minutes if telnet mode and no data sent:
  - **NOP** (default): Sends telnet NOP command (IAC NOP)
  - **Custom**: Sends user-defined command (configured in keep_alive_cmd)
  - **Generic**: Sends "help commands ##_idler_message_<rand>_###" (works on most MUDs)
- Timing fields (last_send_time, last_receive_time) initialized on connect and after /reload for proper NOP tracking

**Supported Telnet Options:**

| Option | Code | Description |
|--------|------|-------------|
| SGA (Suppress Go Ahead) | 3 | Accepts server's WILL SGA with DO SGA |
| TTYPE (Terminal Type) | 24 | Reports terminal type from TERM env var (default: "ANSI") |
| EOR (End of Record) | 25 | Alternative prompt marker, treated same as GA |
| NAWS (Window Size) | 31 | Reports minimum output dimensions across all connected clients |

**NAWS Behavior:**
- Sends smallest width × height across console + all web/GUI clients
- Updates sent when terminal resizes or client dimensions change
- Dimensions tracked per-world, reset on disconnect

**TTYPE Behavior:**
- Responds to SB TTYPE SEND with SB TTYPE IS <terminal>
- Uses TERM environment variable (e.g., "xterm-256color")
- Falls back to "ANSI" if TERM is not set

### Telnet Prompt Detection (GA/EOR)

- When telnet GA (Go Ahead, IAC GA) or EOR (End of Record, IAC EOR) is received, the text from the last newline is identified as a prompt
- The prompt is stored per-world and displayed at the start of the input area (styled in cyan)
- The prompt is NOT shown in the output area, only in the input area
- When the user sends a command, the prompt is cleared
- Prompts are world-specific and only shown for the current world
- Prompt trailing spaces are normalized: all trailing spaces stripped, then exactly one space added
- Cursor positioning uses visible prompt length (ANSI codes stripped) to place cursor correctly
- After prompt extraction, data containing only newlines is discarded (prevents blank lines in output)

### Line Buffering

- Incoming TCP data is buffered to prevent splitting ANSI escape sequences
- `find_safe_split_point()` checks for incomplete ANSI CSI sequences and telnet commands
- Incomplete ANSI/telnet sequences remain buffered until the next read completes them
- Remaining buffer is flushed on connection close

**Partial Line Handling:**
- Lines without trailing newlines (e.g., prompts) are displayed immediately
- `partial_line` tracks incomplete lines, `partial_in_pending` tracks which list they're in
- When more data arrives continuing a partial line, the existing line is updated in-place
- Prevents duplicate lines when data is split across TCP reads
- Correctly handles partials in both `output_lines` and `pending_lines` (when paused)

### Async Architecture

- Reader task per world: reads raw bytes via StreamReader, sends `AppEvent::ServerData(world_idx, bytes)`
- Writer task per world: receives commands via channel, writes via StreamWriter
- Main loop: handles terminal events, decodes bytes with current encoding, routes to world

### Files

**Core:**
- `src/main.rs` - Main application (TUI, event loop, world management, connections)
- `src/encoding.rs` - Character encoding (UTF-8, Latin1, Fansi), colored emoji handling, and console `Theme` enum (Dark/Light with hardcoded ratatui colors)
- `src/input.rs` - Input area with viewport scrolling, cursor positioning, display width helpers
- `src/actions.rs` - Action/trigger system (pattern matching, command execution, capture groups)
- `src/telnet.rs` - Telnet protocol negotiation and option handling
- `src/spell.rs` - Spell checking with Levenshtein distance suggestions
- `src/persistence.rs` - Settings save/load (`~/.clay.dat` INI format)
- `src/util.rs` - Shared utility functions
- `src/daemon.rs` - Daemon/background process management
- `src/ansi_music.rs` - ANSI music sequence parsing and playback

**Networking:**
- `src/websocket.rs` - WebSocket server, message types, client management
- `src/http.rs` - HTTP/HTTPS web server (3 handler implementations: native-tls, rustls, plain)

**Theme:**
- `src/theme.rs` - `ThemeColors` struct (42 customizable color vars), `ThemeFile` for loading `~/clay.theme.dat` (INI format). Used by GUI/web only; console uses `Theme` from encoding.rs

**Popup System:**
- `src/popup/mod.rs` - Unified popup system (PopupManager, field types, layout)
- `src/popup/console_renderer.rs` - Console/TUI popup rendering (ratatui)
- `src/popup/gui_renderer.rs` - GUI popup rendering (egui)
- `src/popup/definitions/` - Individual popup definitions (actions, confirm, connections, filter, help, menu, setup, web, world_editor, world_selector)

**TF (TinyFugue) Engine:**
- `src/tf/mod.rs` - TF engine state, variable storage, macro registry
- `src/tf/parser.rs` - Command parsing, routing (`#`/`/` prefix), inline block detection
- `src/tf/macros.rs` - Macro definition, trigger matching, execution, attribute parsing
- `src/tf/builtins.rs` - Built-in TF commands (`/echo`, `/set`, `/load`, `/save`, `/quote`, etc.)
- `src/tf/control_flow.rs` - `/if`/`/while`/`/for` block execution
- `src/tf/expressions.rs` - Expression evaluator (arithmetic, string, regex, function calls)
- `src/tf/variables.rs` - Variable substitution (`%{var}`, `%Pn`, positional params)
- `src/tf/hooks.rs` - Hook event system (CONNECT, DISCONNECT, LOAD, etc.)
- `src/tf/bridge.rs` - Bridge between TF engine and App (command results → app actions)

**Remote GUI:**
- `src/remote_gui.rs` - Remote GUI client (egui, WebSocket connection)

**Web Interface:**
- `src/web/index.html` - Web interface HTML template
- `src/web/style.css` - Web interface CSS styles (uses `var(--theme-*)` CSS variables)
- `src/web/app.js` - Web interface JavaScript client
- `src/web/theme-editor.html` - Standalone browser-based theme color editor with live preview

**Testing:**
- `src/testharness.rs` - Test harness for integration tests
- `src/testserver.rs` - Mock MUD server for testing
- `src/bin/clay-test-server.rs` - Standalone test server binary

**Data Files:**
- `clay2.png` - Logo image used in remote GUI login screen and Android app icon
- `android/app/src/main/res/mipmap-*/` - Android launcher icons (generated from clay2.png)
- `websockets.readme` - WebSocket protocol documentation
- `~/.clay.dat` - Settings file (created on first save)
- `~/clay.theme.dat` - Theme file (INI format with `[theme:name]` sections, editable via theme editor)
- `/usr/share/dict/words` - System dictionary for spell checking (fallback: american-english, british-english)

### Controls

**World Switching:**
- `Up/Down` - Cycle through active worlds (connected OR with unseen output)
- `Escape` then `w` - Switch to world with activity (priority: oldest pending → unseen output → previous world)
- `Shift+Up/Down` - Cycle through all worlds that have ever been connected
- Disconnected worlds without unseen lines are skipped from Up/Down cycling

World switching behavior is controlled by the "World Switching" setting:

1. **Unseen First**: If any OTHER world has unseen output, switch to the world that received unseen output first (oldest unseen). Done.
2. **Alphabetical** (or when no unseen): Switch to the alphabetically next world by name. Wraps from the last world back to the first.

This logic applies to all interfaces (console, web, GUI). Remote clients query the master instance for consistent world switching across all views.

**Input Area:**
- `Left/Right` or `Ctrl+B/Ctrl+F` - Move cursor
- `Ctrl+Up/Down` - Move cursor up/down in multi-line input
- `Alt+Up/Down` - Resize input area (1-15 lines)
- `Ctrl+U` - Clear input
- `Ctrl+W` - Delete word before cursor
- `Ctrl+P/N` - Previous/Next command history
- `Ctrl+Q` - Spell suggestions / cycle and replace
- `Ctrl+A` or `Home/End` - Jump to start/end (Ctrl+A = start only)
- `Tab` - Command completion (when input starts with `/` or `#`); more-mode takes priority if paused

**Output Scrollback:**
- `PageUp` - Scroll back in history (enables more-pause)
- `PageDown` - Scroll forward (unpauses if at bottom)
- `Tab` - Release one screenful of pending lines (when paused); scroll down like PgDn (when viewing history)
- `Escape` then `j` - Jump to end, release all pending lines
- `Alt+w` (or `Escape` then `w`) - Switch to world with activity (priority: oldest pending → unseen output → previous world)
- `F4` - Open filter popup to search output

**General:**
- `F1` - Open help popup
- `F2` - Toggle MUD tag display (show/hide tags like `[channel:]` and timestamps)
- `F8` - Toggle action pattern highlighting (highlight lines matching action patterns without running commands)
- `F9` - Toggle GMCP media audio (master mute switch, starts muted)
- `Ctrl+C` - Press twice within 15 seconds to quit
- `Ctrl+L` - Redraw screen (filters out client-generated output, keeps only MUD server data)
- `Ctrl+R` - Hot reload (same as /reload)
- `Ctrl+Z` - Suspend process (use `fg` to resume)
- `/quit` - Exit the client
- `Enter` - Send command

**Popup Controls (unified popup system):**
- `Up/Down` - Navigate between fields (auto-enters edit mode for text fields)
- `Tab/Shift+Tab` - Cycle through buttons only
- `Left/Right` - Navigate between buttons (when on button row); change select/toggle values
- `Enter` - Edit text field / Toggle option / Activate button
- `Space` - Toggle boolean / Cycle options
- `S/C/D/O` - Shortcut keys for Save/Cancel/Delete/Connect buttons (when available)
- `Esc` - Close popup or cancel text edit
- Text fields: Just start typing to edit (inline editing with cursor)
- Buttons have highlighted shortcut letters
- Popups size dynamically based on content

**Mouse Controls (when Console Mouse enabled in /setup, default: on):**
- Left click on popup buttons to activate them
- Left click on popup fields to select and edit/toggle them
- Left click on list items to select them
- Click and drag in scrollable content or list fields to highlight lines of text
- Any keyboard input clears the highlight

### Commands

- `/help` - Show help popup (90% terminal width, scrollable, word-wrapped)
- `/disconnect` (or `/dc`) - Disconnect current world and close log file
- `/send [-W] [-w<world>] [-n] <text>` - Send text to world(s)
  - `-w<world>` - Send to specified world (by name)
  - `-W` - Send to all connected worlds
  - `-n` - Send without end-of-line marker (CR/LF)
  - No flags: Send to current world
- `/setup` - Open Global Settings popup (more mode, spell check, temp convert, world switching, show tags, input height, themes, TLS proxy)
- `/web` - Open Web Settings popup (HTTP/HTTPS servers, WebSocket settings, TLS configuration)
- `/worlds` - Open World Selector popup (list all worlds, filter, connect or edit)
- `/worlds <name>` - Connect to world if exists (opens editor if no hostname/port configured), otherwise create and open editor
- `/worlds -e [name]` - Open World Settings editor for current world or specified world (creates if needed)
- `/worlds -l <name>` - Connect to world without sending auto-login credentials
- `/connections` (or `/l`) - List connected worlds in table format with columns:
  - **World**: World name (`*` = current)
  - **Unseen**: Count of unseen lines (empty if 0)
  - **LastSend**: Time since last user command sent
  - **LastRecv**: Time since last data received from server
  - **LastNOP**: Time since last NOP keepalive was sent
  - **NextNOP**: Time until next NOP keepalive
- `/reload` - Hot reload: exec new binary while preserving TCP connections
- `/testmusic` - Play a test ANSI music sequence (C-D-E-F-G) to verify audio works
- `/notify <message>` - Send notification to Android app (works from input or action commands)
- `/quit` - Exit the client

### TF Commands (TinyFugue Compatibility)

Clay includes a TinyFugue compatibility layer. TF commands work with both `/` and `#` prefixes (unified command system).

**Unified Command System:**
- All TF commands now work with `/` prefix: `/set`, `/echo`, `/def`, etc.
- The `#` prefix still works for backward compatibility: `#set`, `#echo`, `#def`, etc.
- Conflicting commands use `/tf` prefix for TF version: `/tfhelp` (TF text help) vs `/help` (Clay popup), `/tfgag` (TF gag pattern) vs `/gag` (Clay action gag)

**Variables:**
- `/set varname value` (or `#set`) - Set global variable
- `/unset varname` - Remove variable
- `/let varname value` - Set local variable (within macro scope)
- `/setenv varname` - Export variable to environment
- `/listvar [pattern]` - List variables matching pattern

**Output:**
- `/echo [-w world] message` - Display local message (supports `%{var}` substitution and ANSI attributes)
  - ANSI attributes: `@{B}` bold, `@{U}` underline, `@{I}` inverse, `@{D}` dim, `@{F}` flash, `@{n}` normal/reset
  - Colors: `@{Crgb}` foreground (r,g,b = 0-5), `@{BCrgb}` background, `@{Cname}` named colors (red, green, blue, cyan, magenta, yellow, white, black)
- `/send [-w world] text` - Send text to MUD
- `/beep` - Terminal bell
- `/quote [options] [prefix] source [suffix]` - Generate and send text from file, command, or literal
  - Sources: `'"file"` (read from file), `` `"command" `` (read internal command output), `!"command"` (read shell output), or literal text
  - Options: `-dsend` (default), `-decho` (display locally), `-dexec` (execute as TF), `-wworld`
  - Example: `/quote say '"/tmp/lines.txt"` sends "say <line>" for each line in file
  - Example: `` /quote think `"/version" `` sends "think Clay v1.0..." to MUD
  - Example: `/quote !"ls -la"` sends output of shell ls command to MUD

**Expressions:**
- `/expr expression` - Evaluate and display result
- `/test expression` - Evaluate as boolean (returns 0 or 1)
- `/eval expression` - Evaluate and execute result as command
- Operators: `+ - * / %` (arithmetic), `== != < > <= >=` (comparison), `& | !` (logical), `=~ !~` (regex), `=/ !/` (glob), `?:` (ternary)
- String functions: `strlen()`, `substr()`, `strcat()`, `tolower()`, `toupper()`, `strstr()`, `replace()`, `sprintf()`, `strcmp()`, `strncmp()`, `strchr()`, `strrchr()`, `strrep()`, `pad()`, `ascii()`, `char()`
- Math functions: `rand()`, `time()`, `abs()`, `min()`, `max()`, `mod()`, `trunc()`, `sin()`, `cos()`, `tan()`, `asin()`, `acos()`, `atan()`, `exp()`, `pow()`, `sqrt()`, `log()`, `log10()`
- Regex: `regmatch(pattern, string)` - Match and populate %P0-%P9 capture groups
- World functions: `fg_world()`, `world_info(field[, world])`, `nactive()`, `nworlds()`, `is_connected([world])`, `idle([world])`, `sidle([world])`, `addworld()`
- Info functions: `columns()`, `lines()`, `moresize()`, `getpid()`, `systype()`, `filename()`, `ftime()`, `nmail()`
- Macro functions: `ismacro(name)`, `getopts(optstring, varname)`
- Command functions: `echo(text[, attrs])`, `send(text[, world])`, `substitute(text[, attrs])`, `keycode(str)`
- Keyboard buffer: `kbhead()`, `kbtail()`, `kbpoint()`, `kblen()`, `kbgoto(pos)`, `kbdel(n)`, `kbmatch()`, `kbword()`, `kbwordleft()`, `kbwordright()`, `input(text)`
- File I/O: `tfopen(path, mode)`, `tfclose(handle)`, `tfread(handle, var)`, `tfwrite(handle, text)`, `tfflush(handle)`, `tfeof(handle)`

**Control Flow:**
- `/if (expr) command` - Single-line conditional
- `/if (expr) ... /elseif (expr) ... /else ... /endif` - Multi-line conditional
- `/while (expr) ... /done` - While loop
- `/for var start end [step] ... /done` - For loop
- `/break` - Exit loop early

**Macros (Triggers):**
- `/def [options] name [= body]` - Define macro (body is optional for attribute-only macros)
  - `-t"pattern"` - Trigger pattern (fires when MUD output matches)
  - `-mtype` - Match type: `simple`, `glob` (default), `regexp`
  - `-p priority` - Execution priority (higher = first)
  - `-F` - Fall-through (continue checking other triggers)
  - `-1` - One-shot (delete after firing once)
  - `-n count` - Fire only N times
  - `-a` - Attributes: supports both single-letter TF codes (`g`=gag, `h`=hilite, `B`=bold, `u`=underline, `r`=reverse, `b`=bell) and long-form names (`"gag"`, `"bold"`, etc.)
  - `-ag` - Gag (suppress) matched line
  - `-ah` - Highlight matched line
  - `-ab` - Bold
  - `-au` - Underline
  - `-E"expr"` - Conditional (only fire if expression is true)
  - `-c chance` - Probability (0.0-1.0)
  - `-w world` - Restrict to specific world
  - `-h event` - Hook event (CONNECT, DISCONNECT, etc.)
  - `-b"key"` - Key binding
- `/undef name` - Remove macro
- `/undefn pattern` - Remove macros matching name pattern
- `/undeft pattern` - Remove macros matching trigger pattern
- `/list [pattern]` - List macros
- `/purge [pattern]` - Remove all macros (or matching pattern)

**Hooks:**
- `/def -hCONNECT name = command` - Fire on connect
- `/def -hDISCONNECT name = command` - Fire on disconnect
- Events: `CONNECT`, `DISCONNECT`, `LOGIN`, `PROMPT`, `SEND`, `ACTIVITY`, `WORLD`, `RESIZE`, `LOAD`, `REDEF`, `BACKGROUND`

**Key Bindings:**
- `/bind key = command` - Bind key to command
- `/unbind key` - Remove binding
- Key names: `F1`-`F12`, `^A`-`^Z` (Ctrl), `@a`-`@z` (Alt), `PgUp`, `PgDn`, `Home`, `End`, `Insert`, `Delete`

**File Operations:**
- `/load filename` - Load TF script file
- `/save filename` - Save macros to file
- `/lcd path` - Change local directory

**World Commands:**
- `/fg [world]` - Switch to specified world (or show current)
- `/addworld [-Lq] name host port` - Create a new world

**Input Commands:**
- `/input text` - Insert text into input buffer at cursor
- `/grab [world]` - Grab last line from world's output into input buffer
- `/trigger [pattern]` - Manually trigger macros matching pattern

**Miscellaneous:**
- `/time` - Display current time
- `/version` - Show TF compatibility version
- `/tfhelp [topic]` - Show TF text help (vs `/help` for Clay popup)
- `/ps` - List background processes
- `/kill id` - Kill background process
- `/repeat [-p priority] time count command` - Schedule repeated command (-p sets priority, higher = runs first)
- `/sh command` - Execute shell command
- `/recall [pattern]` - Search output history

**Variable Substitution:**
- `%{varname}` - Variable value
- `%varname` - Variable value (simple form)
- `%1` - `%9` - Positional parameters from trigger match
- `%*` - All positional parameters
- `%L` - Text left of match
- `%R` - Text right of match
- `%P0` - `%P9` - Regex capture groups from `regmatch()` or trigger match (%P0 = full match)
- `%%` - Literal percent sign
- `\%` - Literal percent sign (backslash escape)

**Capture Groups in Expressions:**
- Trigger capture groups are available in both text substitution (`%P1`) and expression context (`{P1}`)
- `{P0}` - Full match, `{P1}`-`{P9}` - Capture groups, `{PL}` - Left of match, `{PR}` - Right of match
- These are set as local variables in macro scope so the expression evaluator can resolve them

**Special Variables:**
- `%{world_name}` - Current world name
- `%{world_host}` - Current world hostname
- `%{world_port}` - Current world port
- `%{world_character}` - Current world username
- `%{pid}` - Process ID
- `%{time}` - Unix timestamp
- `%{version}` - TF compatibility version string
- `%{nworlds}` - Total number of worlds
- `%{nactive}` - Number of connected worlds

**Example - Auto-heal trigger:**
```
/def -t"Your health: *" -mglob heal_check = /if ({1} < 50) cast heal
```

**Example - Connect hook:**
```
/def -hCONNECT auto_look = look
```

### ANSI Music

The client supports ANSI music sequences (BBS-style PC speaker music). When enabled, music sequences are extracted from MUD output and played through the web interface or remote GUI.

**Format:** `ESC [ M <music_string> Ctrl-N` or `ESC [ N <music_string> Ctrl-N`
- Also supports `MF` (foreground) and `MB` (background) modifiers
- Music string uses BASIC PLAY command syntax: notes (A-G), octave (O, <, >), tempo (T), length (L)

**Configuration:**
- Toggle "ANSI Music" in Setup (`/setup`)
- Use `/testmusic` to verify audio is working

**Playback:**
- Web interface: Uses Web Audio API with square wave oscillator
- Remote GUI: Uses rodio library (requires `remote-gui-audio` feature and ALSA dev libraries)
- Console: No audio playback (music sequences are stripped from display)

**Building with audio support:**
```bash
cargo build --features remote-gui-audio  # Requires libasound2-dev on Linux
```

### GMCP Media (MCMP Protocol)

The client supports GMCP-based media playback using the Client.Media.* protocol (MCMP). MUD servers can send sound effects and music to be played locally.

**Supported GMCP Packages:**
- `Client.Media.Default` - Sets default URL for resolving relative media paths
- `Client.Media.Play` - Play a sound effect or music track
- `Client.Media.Stop` - Stop playing a sound by key or type
- `Client.Media.Load` - Pre-cache a media file without playing

**F9 Master Mute Switch:**
- F9 toggles audio on/off (starts muted/disabled)
- Media state (`active_media`) is always tracked regardless of F9 state
- When F9 enables: active media for the current world starts playing
- When F9 disables: all running media processes are killed
- World switching handles per-world play/stop, but only when F9 is enabled

**World Switching Media Behavior:**
- Switching away from a world stops its media processes
- Switching back restarts active looping media (if F9 is enabled)
- Looping media (loops=-1 or loops>1) is tracked in `active_media` per world
- Web/GUI clients also receive media restart on world switch

**Console Playback:**
- Uses ffplay or mpv (auto-detected at startup)
- Media files are cached in `~/.clay_media_cache/`
- Download and playback happen in background threads
- Child process handles delivered via `AppEvent::MediaProcessReady` for reliable tracking

**Implementation:**
- `handle_gmcp_media(world_idx, package, json_data, play_audio)` - Central handler; always tracks state, only spawns processes when `play_audio=true`
- `active_media: HashMap<String, String>` on World - Maps key to Play JSON for restart
- `media_processes: HashMap<String, (usize, Child)>` on App - Running player processes
- `stop_world_media(world_idx)` - Kill all processes for a world
- `restart_world_media(world_idx)` - Replay active_media entries (checks gmcp_user_enabled)
- `ws_send_active_media_to_client()` - Send active media to a specific WebSocket client

### Hot Reload

The `/reload` command performs a true hot code reload:

1. Saves complete application state (output buffers, pending lines, scroll positions, per-world settings including auto login type, etc.)
2. Clears FD_CLOEXEC on socket file descriptors so they survive exec
3. Calls exec() to replace the current process with a fresh binary
4. The new process detects reload mode and restores state including all output history
5. TCP socket connections are reconstructed from the preserved file descriptors
6. Cleanup pass runs to fix any inconsistent world states:
   - Worlds claiming to be connected but without a working command channel are marked disconnected
   - Pending lines are cleared for all disconnected worlds (only meaningful for active connections)
   - Paused state is cleared for disconnected worlds
7. Restored connections have auto-login disabled (only fresh connections trigger auto-login)

Works with updated binaries: On Linux, when the binary is rebuilt while running, `/proc/self/exe` shows a " (deleted)" suffix. The reload logic strips this suffix to find the new binary, allowing seamless reload after `cargo build` without restarting.

**Triggering reload:**
- `/reload` command from within the client
- `Ctrl+R` keyboard shortcut
- Send SIGUSR1 signal: `kill -USR1 $(pgrep clay)`

**Limitations:**
- TLS/SSL connections: By default, TLS connections cannot be preserved (TLS state is in userspace) and will need manual reconnection after reload
- Enable "TLS Proxy" in `/setup` to preserve TLS connections across reload (uses forked proxy process)
- Requires the new binary to be compatible with the state format

**Use cases:**
- Apply code changes without losing active MUD sessions
- Debug fixes can be deployed without reconnecting
- External scripts can trigger reload via SIGUSR1

**Message suppression:**
During hot reload, success messages are suppressed to reduce noise:
- WebSocket/HTTP/HTTPS server startup messages (only shown on failure)
- Binary path message (only shown on failure)
- Warnings and errors are always shown

### Crash Handler

The client includes automatic crash recovery:

- On panic, saves application state and attempts to restart
- Maximum 2 restart attempts to prevent infinite crash loops
- Terminal state is restored before showing crash info
- TCP connections are preserved across restarts (same as hot reload)
- Crash count is cleared after first successful user input
- Uses `--crash` flag to distinguish crash restarts from normal reloads

### Settings Persistence

- Settings automatically loaded from `~/.clay.dat` on startup
- Settings automatically saved when closing settings popup (Save button, Enter, or Ctrl+S)
- File format is INI-like with `[global]` and `[world:name]` sections
- Hot reload state saved temporarily to `~/.clay.reload`

### WebSocket Server

The client includes an embedded WebSocket server that allows remote GUI clients to connect and control the MUD sessions.

**Configuration (in Web Settings - /web):**
- `WS enabled` - Enable/disable the WebSocket server
- `WS port` - Port to listen on (default: 9002)
- `WS password` - Password required for authentication (stored encrypted)

**Server Behavior:**
- Starts on startup/reload if enabled and password is set
- Stops when disabled or password is cleared
- Listens on 0.0.0.0 (all interfaces) on the configured port
- Requires password authentication before sending any data
- Multiple clients can connect simultaneously
- On auth, clients receive `InitialState` with output_lines only (pending_lines are NOT merged to avoid duplicates)
- Clients see pending line count via `pending_count` field and release via PgDn/Tab (released lines broadcast as ServerData)
- All MUD data is broadcast to authenticated clients in real-time
- Keepalive: clients send Ping every 30s, server responds with Pong

**Protocol:**
- JSON over WebSocket
- Message types: AuthRequest, AuthResponse, InitialState, ServerData, WorldConnected, WorldDisconnected, WorldSwitched, PromptUpdate, SendCommand, SwitchWorld, ConnectWorld, DisconnectWorld, MarkWorldSeen, UnseenCleared, Ping, Pong
- Password is hashed with SHA-256 before transmission

**Cross-Interface Sync:**

- Console marks a world as seen when its output is rendered (clears `unseen_lines` to 0)
- `UnseenCleared` message broadcast to all clients when a world is marked as seen
- `UnseenUpdate` message sent when a world's unseen count changes
- `ActivityUpdate` message broadcast when overall activity count changes (console is authoritative)
- Web/GUI clients send `MarkWorldSeen` when switching to a world
- All interfaces stay synchronized via these broadcasts
- Activity indicator shows same count across console, web, and GUI (server broadcasts count, clients display it)

**Client World Tracking:**

- Server tracks which world each client is viewing via `WsClient::current_world`
- Set immediately on authentication to the server's current world (eliminates race condition)
- Updated when client sends `UpdateViewState` or `MarkWorldSeen` messages
- Used by `broadcast_to_world_viewers()` to route output only to clients viewing that world
- Client-generated messages (connection errors, etc.) are broadcast via `ws_broadcast_to_world()`

**Allow List Whitelist:**

The WebSocket server supports dynamic whitelisting based on the allow list:

- `WS allow list` - CSV list of IPs that *can* be whitelisted (not auto-authenticated)
- Empty allow list = always require password for all connections
- Whitelisting happens when a client from an allow-list IP authenticates with password:
  - That IP gets whitelisted for future connections (auto-authenticated)
  - Any previously whitelisted IP is cleared (only ONE whitelisted IP at a time)
- Non-allow-list IPs can authenticate with password but are never whitelisted

**Behavior:**
1. Client connects from IP in allow list → must authenticate with password first time
2. After successful auth → IP is whitelisted
3. Future connections from that IP → auto-authenticated (no password needed)
4. Different allow-list IP authenticates → previous whitelist cleared, new IP whitelisted
5. Client connects from IP NOT in allow list → must always authenticate with password

**Use case:** Allows trusted IPs (e.g., home network) to authenticate once, then reconnect without password. Moving to a new location and authenticating clears the old whitelist.

### HTTP/HTTPS Web Interface

A browser-based client that connects via WebSocket to control MUD sessions.

**Configuration (in Web Settings - /web):**
- `HTTP enabled` - Enable/disable the HTTP web server (uses ws://)
- `HTTP port` - Port for HTTP server (default: 9000)
- `HTTPS enabled` - Enable/disable the HTTPS web server (uses wss://)
- `HTTPS port` - Port for HTTPS server (default: 9001)
- Reuses TLS cert/key from WebSocket settings for HTTPS

Note: HTTP automatically starts the non-secure WebSocket server if not already running.

**Features:**
- Full MUD client in the browser
- ANSI color rendering (Xubuntu Dark palette, 256-color and true color support)
- Shade character blending (░▒▓ rendered as solid blocks with blended colors)
- Clickable URLs in output (cyan, underlined, opens in new tab)
- More-mode pausing with Tab or Escape+j
- Command history (Ctrl+P/N)
- Multiple world support with world switching
- World selector popup (`/worlds` command)
- Connected worlds list (`/connections` or `/l` command) with arrow key navigation
- Actions popup (`/actions` command)
- MUD tag stripping toggle (F2)
- Keep-alive/idler message filtering (same as console)
- Independent view from console (world switching is local)
- Text selection for copying (right-click for browser context menu)
- Toolbar with hamburger menu and font size controls
- Cross-interface unseen indicator sync (console, web, GUI stay in sync)

**Web Toolbar:**
- Hamburger menu (SVG icon) in upper left with dropdown:
  - Worlds List - Opens connected worlds list popup
  - World Selector - Opens world selector popup
  - Actions - Opens actions editor popup
  - Settings - Opens settings popup
  - Toggle Tags - Show/hide MUD tags (same as F2)
  - Toggle Highlight - Highlight lines matching action patterns (same as F8)
  - Resync - Request full state resync from server
  - Clay Server - Disconnect and open server settings (Android app only)
- Font slider next to hamburger for adjusting text size
- "Clay" title displayed after font slider

**Mobile Web Interface:**
- Toolbar uses `position: fixed` to stay visible during scrolling and keyboard display
- Uses `100dvh` (dynamic viewport height) instead of `100vh` for proper mobile sizing
- Visual Viewport API tracks keyboard appearance and adjusts toolbar position
- `overscroll-behavior: contain` prevents scroll chaining to parent elements
- `-webkit-overflow-scrolling: touch` for smooth iOS scrolling
- `interactive-widget=resizes-content` viewport meta for proper keyboard handling
- Hamburger icon uses inline SVG for reliable cross-browser rendering
- Wake-from-background health check: sends Ping on visibility change, waits 3s for Pong, auto-reconnects if stale

**Mobile Toolbar Layout:**
- Left side: Menu (hamburger), PgUp, PgDn
- Right side: ▲ (Previous World), ▼ (Next World)
- PgUp/PgDn on left for easy scrolling access; world switching on right

**Web Interface Controls:**
- `Up/Down` - Switch between active worlds
- `PageUp/PageDown` - Scroll output history
- `Tab` - Release one screenful when paused; scroll down one screenful otherwise (like `more`)
- `Escape+j` - Jump to end, release all pending
- `Ctrl+P/N` - Command history navigation
- `Ctrl+U` - Clear input
- `Ctrl+W` - Delete word before cursor
- `Ctrl+A` - Move cursor to beginning of line
- `Alt+Up/Down` - Resize input area
- `F2` - Toggle MUD tag display (show/hide tags and timestamps)
- `F4` - Open filter popup to search output
- `F8` - Toggle action pattern highlighting
- `F9` - Toggle GMCP media audio (master mute switch)
- `Enter` - Send command

**Web Popup Controls:**
- `/worlds` - World selector with filter, arrow keys to navigate, Enter to switch
- `/connections` or `/l` - Connected worlds list with arrow keys to navigate, Enter to switch
- `/actions` - Actions editor
- `Escape` - Close any popup

**Files:**
- `src/web/index.html` - HTML template
- `src/web/style.css` - CSS styles
- `src/web/app.js` - JavaScript client

### Android Notifications

The Android app supports native notifications triggered by the `/notify` command.

**Usage:**
- `/notify Someone is attacking!` - Sends a notification with the message
- Can be used in action commands: `/notify Page from $1`

**Features:**
- Notifications appear even when the app is in background
- Tapping notification opens the app
- Foreground service keeps WebSocket connection alive

**Foreground Service:**
When authenticated, the app starts a foreground service that:
- Shows a persistent "Connected to MUD server" notification
- Keeps the WebSocket connection alive in background
- Allows notifications to be received even when app is not active
- Automatically stops when disconnected

**Action Example:**
```
Name: page_alert
Pattern: *pages you*
Command: /notify Page received: $0
```

### Remote GUI Client

A graphical client mode that connects to a running Clay's WebSocket server.

**Building:**
```bash
cargo build --features remote-gui    # Requires X11 or Wayland
```

**Running:**
```bash
./clay --remote=hostname:port
```

**Features:**
- Graphical window using egui
- Login screen displays clay2.png logo (200x200) with password entry
- Authentication follows allow list and whitelisting rules (same as web interface)
- World tabs at top showing connection status (● connected, ○ disconnected)
- Scrollable output area with ANSI color support
- Clickable URLs in output (underlined, pointer cursor on hover, click to open browser)
- Input field at bottom with prompt display
- Status bar showing connection state
- Click world tabs to switch between worlds
- Send commands to any connected world
- Command history with Ctrl+P/N navigation
- MUD tag stripping toggle (F2)
- Output filtering (F4)
- World switching is GUI-local (doesn't affect console)
- Hamburger menu with: Worlds List, World Selector, World Editor, Setup, Font, Toggle Tags, Toggle Highlight, Resync
- Debug Selection: Right-click highlighted text to see raw ANSI codes (ESC shown as `<esc>`)

**Limitations:**
- The remote-gui feature cannot be built in headless environments
- Requires X11 or Wayland display server

### Remote Console Client

A terminal-based client mode that connects to a running Clay's WebSocket server, providing the same interface as the main console but connecting remotely.

**Running:**
```bash
./clay --console=hostname:port
```

**Features:**
- Full terminal interface identical to the main console
- Connects via WebSocket to a master Clay instance
- All popup dialogs work (help, menu, setup, world selector, world editor, actions, web settings)
- Output scrollback with PageUp/PageDown
- More-mode pausing with Tab
- World switching with Up/Down arrows
- Command history with Ctrl+P/N
- MUD tag stripping toggle (F2)
- Output filtering (F4)
- Action pattern highlighting (F8)

**Key Differences from Main Console:**
- No direct MUD connections - all data flows through the master instance
- World switching is console-local (doesn't affect master or other clients)
- Commands are sent to the master for execution
- Popups are rendered locally using the unified popup system

**Keyboard Shortcuts:**
- All standard console shortcuts work (see Controls section)
- `Ctrl+L` - Redraw screen (filters out client-generated output, keeps only server data)
- `/menu` - Open hamburger menu popup
- `/version` - Display version information

**Building:**
```bash
# No special features required - works with standard musl build
cargo build --target x86_64-unknown-linux-musl --no-default-features --features rustls-backend
```

### GUI Keyboard Shortcuts

The remote GUI client supports keyboard shortcuts similar to the console client:

**World Switching:**
- `Up/Down` - Cycle through active worlds (follows world switching rules from master)
- `Shift+Up/Down` - Cycle through all worlds

**Input Area:**
- `Ctrl+U` - Clear input buffer
- `Ctrl+W` - Delete word before cursor
- `Ctrl+A` - Move cursor to beginning of line
- `Ctrl+P/N` - Previous/Next command history
- `Ctrl+Up/Down` - Move cursor up/down in multi-line input
- `Alt+Up/Down` - Resize input area (1-15 lines)
- `Enter` - Send command

**Output Scrollback:**
- `PageUp/PageDown` - Scroll output up/down

**Display:**
- `F2` - Toggle MUD tag display (show/hide `[channel:]` tags and timestamps)
- `F4` - Open filter popup to search output
- `F8` - Toggle action pattern highlighting
- `F9` - Toggle GMCP media audio (master mute switch)
- `Esc` or `F4` - Close filter popup

**Menu Shortcuts:**
- `Ctrl+L` - Open World List popup
- `Ctrl+E` - Open World Editor for current world
- `Ctrl+S` - Open Setup popup
- `Ctrl+O` - Connect current world
- `Ctrl+D` - Disconnect current world

**Filter Popup (F4):**
- Type text to filter output (case-insensitive)
- `Esc` or `F4` - Close and clear filter

**Debug Selection (Right-click menu):**
- Highlight text in output area, then right-click
- Select "Debug Selection" from context menu
- Opens popup showing raw text with ANSI escape codes visible
- Escape character (0x1B) displayed as `<esc>` for readability
- Useful for debugging color code issues or unexpected formatting
- Copy button to copy the raw text to clipboard

### World Selector Popup

Opened with `/worlds` command (no arguments):

- Shows list of all worlds with columns: World name, Hostname, Port, User
- Filter box at top - type to filter worlds by name, hostname, or user
- Current world marked with `*`, connected worlds shown in green
- Selected world highlighted in yellow

**Controls:**
- `Up/Down` - Navigate world list (when list focused)
- `Tab` - Cycle focus: List -> Add -> Edit -> Connect -> Cancel
- `Shift+Tab` - Cycle focus backwards
- `Enter` - Activate focused element (connect from list, or activate button)
- `Left/Right` - Move between buttons (when button focused)
- `Esc` - Close popup
- Type any character - Start filtering (when list focused)
- `/` - Focus filter box

**Buttons:**
- Add - Create new world and open editor
- Edit - Open editor for selected world
- Connect - Switch to and connect selected world
- Cancel - Close popup

### World Editor Popup

Opened with `/worlds -e` command (uses unified popup system):

**Per-World Settings:**
- World name - Display name for this world
- Hostname - Server address
- Port - Server port
- User - Username for auto-login
- Password - Password for auto-login (plaintext)
- Use SSL - Enable TLS connection
- Log file - Path to output log file
- Encoding - Character encoding (utf8, latin1, fansi)
- Auto login - Login method (Connect, Prompt, MOO_prompt)
- Keep alive - Keepalive type (NOP, Custom, Generic)
- Keep alive cmd - Custom command for keepalive (only shown when keep alive is Custom)

**Global Settings (via /setup):**
- More mode - Enable/disable more-style pausing
- Spell check - Enable/disable spell checking
- Temp convert - Enable/disable temperature conversion in input (requires F2/show_tags mode)
- World Switching - World cycling behavior (Unseen First, Alphabetical)
- Show tags - Show/hide MUD tags at start of lines (default: hidden)
- Input height - Default input area height (1-15 lines)
- Console Theme - Theme for console interface (dark, light) — uses hardcoded `Theme` enum from `encoding.rs`
- GUI Theme - Theme for remote GUI/web client (dark, light) — uses customizable `ThemeColors` from `theme.rs` / `~/clay.theme.dat`
- Console Mouse - Enable mouse support in console popups (default: on) — click buttons/fields, select list items, drag to highlight text
- TLS Proxy - Enable TLS proxy for preserving TLS connections across hot reload

**Actions (right-aligned buttons):**
- Save - Save settings and close popup
- Cancel - Close popup without saving
- Delete - Delete the world (shows confirmation dialog)
- Connect - Connect to world using current settings (saves first)

### Web Settings Popup

Opened with `/web` command:

**WebSocket (secure wss://):**
- WS enabled - Enable/disable secure WebSocket server
- WS port - Port for secure WebSocket (default: 9002)
- WS password - Password for authentication (required, encrypted storage)
- WS Allow List - CSV list of IPs that can be whitelisted
- TLS Cert File - Path to TLS certificate file
- TLS Key File - Path to TLS private key file
- WS Use TLS - Enable TLS for secure WebSocket

**WebSocket (non-secure ws://):**
- WS Nonsecure - Enable/disable non-secure WebSocket server
- WS NS port - Port for non-secure WebSocket (default: 9003)

**HTTP/HTTPS Web Interface:**
- HTTP enabled - Enable/disable HTTP web server
- HTTP port - Port for HTTP server (default: 9000)
- HTTPS enabled - Enable/disable HTTPS web server
- HTTPS port - Port for HTTPS server (default: 9001)

Note: HTTP server automatically starts the non-secure WebSocket server (ws://) if not already running.

**Controls:**
- `Up/Down` - Navigate between fields (stops at top/bottom, no wrap)
- `Tab/Shift+Tab` - Cycle through buttons only (Save, Cancel)
- `Left/Right` - Toggle between Save and Cancel buttons
- `Enter` - Edit text field / Toggle option / Activate button
- `Space` - Toggle boolean options
- `S` - Select Save button
- `C` - Select Cancel button
- `Ctrl+S` - Save and close
- `Esc` - Close popup

### Help Popup

Opened with `F1` or `/help`:

- Displays 90% of terminal width, centered
- Two-column format with word wrapping that preserves column alignment
- Scrollable content with scrollbar

**Controls:**
- `Up/Down` - Scroll one line
- `PageUp/PageDown` - Scroll multiple lines
- `O` - Highlight Ok button
- `Enter` - Close popup
- `Esc` - Close popup

### Filter Popup

Opened with `F4`:

- Small popup in upper right corner for entering filter text
- Output view shows only lines matching the filter (case-insensitive substring search)
- ANSI color codes are stripped for matching but preserved in display
- Filtered results can be scrolled with PageUp/PageDown

**Controls:**
- Type text - Filter output to matching lines
- `Backspace/Delete` - Edit filter text
- `Left/Right` or `Ctrl+B/Ctrl+F` - Move cursor in filter text
- `Home/End` - Jump to start/end of filter text
- `PageUp/PageDown` - Scroll through filtered results
- `Esc` or `F4` - Close filter and restore normal output view

### MUD Tag Display

Toggle with `F2`:

- MUD tags are prefixes like `[channel:]` or `[channel(player)]` at the start of lines
- When hidden (default), tags are stripped from display but preserved in buffer
- When shown, full lines including tags are displayed with timestamps
- **Timestamps**: Each line shows when it was received
  - Format: `HH:MM>` for lines from today
  - Format: `DD/MM HH:MM>` for lines from previous days
  - Displayed in cyan before each line
- **Gagged lines**: Lines hidden by `/gag` action command are also shown with F2
- **Temperature conversion**: When enabled in `/setup`, temperatures typed in input are auto-converted (e.g., "32F " → "32F(0C) "). Only active when F2/show_tags mode is on.
- Works in console, GUI, and web interfaces
- Setting persists across sessions in `~/.clay.dat`

### Actions

Actions are automated triggers that match incoming MUD output against patterns and execute commands.

**Action Processing:**
- Incoming lines are checked against all action patterns as they arrive
- Works with all world types: MUD, Slack, and Discord
- Pattern matching is case-insensitive
- ANSI color codes are stripped before pattern matching
- World-specific actions only match for their configured world (empty = all worlds)
- When a pattern matches, the action's commands are executed (sent to the server)

**Match Types:**
- Each action has a configurable match type: **Regexp** (default) or **Wildcard**
- **Regexp**: Pattern is interpreted as a regular expression (e.g., `^You say` matches lines starting with "You say")
- **Wildcard**: Pattern uses glob-style matching where `*` matches any sequence of characters and `?` matches any single character (e.g., `*tells you*` matches any line containing "tells you")
- Wildcard patterns automatically escape regex special characters, making them safer for simple text matching
- Use `\*` and `\?` to match literal asterisk and question mark characters (e.g., `*what\?*` matches lines containing "what?")
- Toggle match type in the action editor (console: Space/Enter/arrows, web/GUI: click button)

**Capture Group Substitution:**
- When a pattern matches, you can use captured text in the commands:
  - `$0` - The entire matched text
  - `$1` through `$9` - Captured groups from the pattern
- For **Regexp** patterns, use parentheses to create capture groups: `^(\w+) tells you: (.*)$`
  - `$1` = the name, `$2` = the message
- For **Wildcard** patterns, each `*` and `?` becomes a capture group automatically
  - Pattern `* tells you: *` with input "Bob tells you: Hello" gives `$1`="Bob", `$2`="Hello"
- Example action: Pattern `* tells you: *` with command `say Thanks, $1!` replies with "Thanks, Bob!"
- When manually invoking actions with `/actionname args`, `$1-$9` are the space-separated args and `$*` is all args

**Commands:**
- Multiple commands can be separated by semicolons
- Special command `/gag` hides the matched line from display (but stores it for F2 viewing)

**Manual Invocation:**
- Actions can be invoked manually by typing `/actionname` in the input
- Actions with empty patterns are manual-only (never trigger automatically)
- Manual invocation works even when disconnected
- Each command is processed individually:
  - Commands starting with `/` are processed as client commands (e.g., `/connect`, `/worlds`)
  - Plain text is sent to the server (shows error per command if not connected)
- `/gag` commands are skipped when invoking actions manually

**Gagging:**
- If an action's command list includes `/gag`, the matched line is hidden
- Gagged lines are stored with a flag and only shown when F2 (show_tags) is enabled
- This allows filtering spam while preserving the ability to review filtered content

**F8 Highlighting:**
- Press F8 to toggle highlighting of lines that match any action pattern
- Matched lines get a dark background color
- Useful for debugging action patterns without running commands

**Startup Actions:**
- Actions can have "Startup" enabled to run their commands when Clay starts
- Fires on fresh start, hot reload (`/reload`, `Ctrl+R`, `SIGUSR1`), and crash recovery
- Commands are split by semicolons and executed individually
- In console mode: `/` commands and `#` (TF) commands are executed; plain text shows a note
- In headless/GUI mode: Only `#` (TF) commands are executed; other commands are skipped
- Useful for loading TF scripts, setting variables, or running initialization commands
- Example: Create an action with empty pattern, Startup enabled, command `#load myconfig.tf`

### Actions List Popup

Opened with `/actions` command:

- Shows list of all actions with columns: Enable status, Name, World, Pattern preview
- Filter box at top - type to filter actions by name, world, or pattern
- Enabled actions show `[✓]`, disabled show `[ ]`

**Controls:**
- `Up/Down` - Navigate action list
- `Space` - Toggle enable/disable for selected action (without opening editor)
- `Enter` - Edit selected action
- `Tab` - Cycle focus between list and buttons
- `A` - Add new action
- `E` - Edit selected action
- `D` - Delete selected action
- `C` - Cancel/close popup
- `F` or `/` - Focus filter box
- `Esc` - Close popup

**Buttons (right-aligned):**
- Add - Create new action and open editor
- Edit - Open editor for selected action
- Delete - Delete selected action (with confirmation)
- Cancel - Close popup

### Action Editor Popup

Opened from actions list (Add or Edit):

**Fields:**
- Name - Action name (also used for manual invocation with `/actionname`)
- World - Restrict to specific world (empty = all worlds)
- Match Type - Regexp or Wildcard (toggle with Space/Enter/arrows)
- Pattern - Trigger pattern (empty = manual-only action)
- Command - Commands to execute (multiline editor, 3 visible lines with scrolling)
- Enabled - Whether the action is active
- Startup - Run commands when Clay starts (including reload/crash recovery)

**Command Field:**
- Multiline editor with 3 visible lines and scrolling viewport
- `Enter` - Insert newline
- `Up/Down` - Move cursor between lines
- Viewport scrolls automatically to keep cursor visible
- Cursor shown as `│` like other text fields

**Controls:**
- `Up/Down` - Navigate between fields
- `Space` - Toggle Enabled field or cycle Match Type
- `Enter` - Edit text field / Toggle option / Activate button
- `Tab` - Cycle to buttons
- `S` - Save shortcut
- `C` - Cancel/close shortcut
- `Esc` - Cancel edit or close popup

**Buttons (right-aligned):**
- Save - Save action and close
- Cancel - Close without saving

### Confirmation Dialog

Appears when deleting a world:
- Shows "Delete world 'name'?" message
- `Left/Right/Up/Down/Tab` - Toggle between Yes/No buttons
- `Y` - Select Yes
- `N` - Select No
- `Enter` - Confirm selection
- `Esc` - Cancel and close dialog
- Cannot delete the last remaining world
- Deletion message includes world name: "World 'name' deleted."

### Spell Checking

Real-time spell checking with misspelled words highlighted in red.

**Dictionary:**
- Uses system dictionary at `/usr/share/dict/words` (fallback: american-english, british-english)
- Words are case-insensitive for checking

**Contraction Support:**
- Contractions like "didn't", "won't", "I'm" are recognized
- Apostrophes between alphabetic characters are treated as part of the word
- Special handling for irregular contractions (e.g., "won't" → "will")

**When Words Are Checked:**
- Words are only checked when "complete" (followed by space, punctuation, or other non-word character)
- Words at end of input are NOT checked while typing (avoids premature flagging)
- Once a word is flagged, it stays flagged until completed again

**Cached Misspelling State:**
- Misspelled word positions are cached between keystrokes
- When editing a word at end of input, the cached state is used instead of re-checking
- This prevents flickering: type "thiss " (flagged) → backspace to "thiss" (stays flagged) → backspace to "this" (stays flagged until you type space, then re-checked and unflagged)

**Controls:**
- `Ctrl+Q` - Cycle through spell suggestions for word at cursor
- Suggestions use Levenshtein distance (max distance 3, limited to similar-length words)

### Command Completion

Tab completion for commands when input starts with `/`:

**Behavior:**
- Press `Tab` when input starts with `/` to cycle through matching commands
- Matches internal commands: `/help`, `/disconnect`, `/dc`, `/send`, `/worlds`, `/connections`, `/setup`, `/web`, `/actions`, `/keepalive`, `/reload`, `/quit`, `/gag`
- Also matches manual actions (actions with empty patterns)
- Completion is case-insensitive
- Pressing `Tab` multiple times cycles through all matches alphabetically
- Arguments after the command are preserved when cycling

### /release Skill

Automated multi-platform build and GitHub release. Invoke with `/release [version]`.

Builds Clay on all target platforms (local Linux musl, Windows cross-compile, Android APK, remote macOS universal, remote Termux aarch64), verifies compilation on a test machine, and uploads release binaries to GitHub.

Skill files: `.claude/skills/release/SKILL.md` (instructions), `.claude/skills/release/machines.md` (machine details).
