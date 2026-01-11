# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build Commands

```bash
cargo build                          # Build debug (dynamically linked)
cargo build --features remote-gui    # Build with remote GUI client (requires X11/Wayland)
cargo run                            # Run the client
cargo run -- --remote=host:port      # Run as remote GUI client
cargo test                           # Run all tests
cargo test test_name                 # Run a single test
cargo clippy                         # Lint
cargo fmt                            # Format code
cargo fmt -- --check                 # Check formatting without changes
```

### Static/Portable Build (musl)

For a portable binary that works on any Linux x86_64 system regardless of glibc version:

```bash
# Install musl target (one-time setup)
rustup target add x86_64-unknown-linux-musl

# Build static binary with musl and rustls (no OpenSSL dependency)
cargo build --target x86_64-unknown-linux-musl --no-default-features --features rustls-backend

# Output: target/x86_64-unknown-linux-musl/debug/clay (or release/)
```

**Why musl instead of glibc static linking:**
- glibc static builds cause SIGFPE crashes during DNS resolution (`getaddrinfo`)
- glibc's NSS (Name Service Switch) requires dynamic loading even in static builds
- musl handles DNS resolution properly in fully static binaries

**Why rustls instead of native-tls:**
- native-tls requires OpenSSL, which needs cross-compilation setup for musl
- rustls is pure Rust and works seamlessly with musl builds

## Architecture

MudClient is a terminal-based MUD (Multi-User Dungeon) client built with ratatui/crossterm for TUI and tokio for async networking. Supports multiple simultaneous world connections with SSL/TLS support.

### Key Structs

- **App**: Central state - worlds list, current world index, input area, spell checker, settings, settings popup
- **World**: Per-connection state - output buffer, scroll position, connection, unseen lines, pause state, world settings, log handle, prompt (from telnet GA)
- **WorldSettings**: Per-world connection settings - hostname, port, user, password, use_ssl, log_file, encoding, auto_connect_type, keep_alive_type, keep_alive_cmd
- **InputArea**: Input buffer with viewport scrolling, command history, cursor position
- **SpellChecker**: System dictionary-based spell checking with Levenshtein-distance suggestions (uses /usr/share/dict/words)
- **Settings**: Global preferences (more_mode_enabled, spell_check_enabled, world_switch_mode, websocket_enabled, websocket_port, websocket_password, https_enabled, https_port)
- **WebSocketServer**: Embedded WebSocket server for remote GUI clients
- **SettingsPopup**: Modal dialog for editing world and global settings with inline text editing and horizontal scrolling for long text fields
- **ConfirmDialog**: Modal confirmation dialog for destructive actions (e.g., delete world)
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

- Status indicator (leftmost, 11 chars, black text on red background):
  - `More: XXXX` - pending lines when paused (priority 1)
  - `Hist: XXXX` - lines scrolled back in history (priority 2)
  - Underscores when neither active (dark gray)
  - Large numbers formatted as: 9999 → "9999", 10000 → " 10K", 1000000+ → "Alot"
- World name (bold white)
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

### Multi-World System

- Each world has independent: output buffer, scroll position, connection, unseen line count
- `unseen_lines` increments when data arrives on non-current world
- Switching worlds calls `mark_seen()` to reset counter

### More-Style Pausing

- Enabled via settings popup (default: on)
- Auto-triggers after `output_height - 2` lines of new output since last user input
- `lines_since_pause` counter tracks lines received; reset to 0 when user sends any command
- When pause triggers, scrolls to bottom first to show buffered output, then pauses
- Also triggered when user scrolls up with PageUp
- Incoming lines queue to `pending_lines` instead of `output_lines`
- Tab releases one screenful of pending lines
- Alt+j releases all pending lines (jump to end)
- Enter releases all pending and sends command
- Scrolling to bottom with PageDown also unpauses

### SSL/TLS Support

- Uses `tokio-native-tls` for TLS connections
- Enable via "Use SSL" toggle in world settings, or `/connect host port ssl`
- `StreamReader`/`StreamWriter` enums wrap both plain TCP and TLS streams
- Implements `AsyncRead`/`AsyncWrite` traits for unified handling

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

Credentials stored per-world. Auto-login only triggers if username is configured.
Prompts that are auto-answered are immediately cleared and not displayed in the input area.

### Telnet Protocol Support

- Automatic telnet negotiation handling
- Responds with WONT for DO requests, DONT for WILL requests
- Strips telnet sequences from displayed output
- Detects telnet mode when IAC sequences are received
- Configurable keepalive sent every 5 minutes if telnet mode and no data sent:
  - **NOP** (default): Sends telnet NOP command (IAC NOP)
  - **Custom**: Sends user-defined command (configured in keep_alive_cmd)
  - **Generic**: Sends "help commands ##_idler_message_<rand>_###" (works on most MUDs)
- Timing fields (last_send_time, last_receive_time) initialized on connect and after /reload for proper NOP tracking

### Telnet GA Prompt Detection

- When telnet GA (Go Ahead, IAC GA) is received, the text from the last newline to GA is identified as a prompt
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

- `src/main.rs` - Main application
- `src/web/index.html` - Web interface HTML template
- `src/web/style.css` - Web interface CSS styles
- `src/web/app.js` - Web interface JavaScript client
- `~/.mudclient.dat` - Settings file (created on first save)
- `/usr/share/dict/words` - System dictionary for spell checking (fallback: american-english, british-english)

### Controls

**World Switching:**
- `Up/Down` - Cycle through active worlds (connected OR with unseen output)
  - "World Switching" setting controls behavior:
    - **Unseen First**: Prioritizes OTHER worlds with unseen output first, then alphabetical
    - **Alphabetical**: Simple alphabetical order by world name
  - "Unseen output" = lines received while viewing another world
  - Disconnected worlds without unseen lines are skipped
- `Alt+w` (or `Escape` then `w`) - Switch to world with activity (priority: oldest pending → unseen output → previous world)
- `Shift+Up/Down` - Cycle through all worlds that have ever been connected

**Input Area:**
- `Left/Right` or `Ctrl+B/Ctrl+F` - Move cursor
- `Ctrl+Up/Down` - Resize input area (1-15 lines)
- `Ctrl+U` - Clear input
- `Ctrl+W` - Delete word before cursor
- `Ctrl+P/N` - Previous/Next command history
- `Ctrl+Q` - Spell suggestions / cycle and replace
- `Ctrl+A` or `Home/End` - Jump to start/end (Ctrl+A = start only)

**Output Scrollback:**
- `PageUp` - Scroll back in history (enables more-pause)
- `PageDown` - Scroll forward (unpauses if at bottom)
- `Tab` - Release one screenful of pending lines (when paused)
- `Alt+j` (or `Escape` then `j`) - Jump to end, release all pending lines
- `Alt+w` (or `Escape` then `w`) - Switch to world with activity (priority: oldest pending → unseen output → previous world)
- `F4` - Open filter popup to search output

**General:**
- `F1` - Open help popup
- `F2` - Toggle MUD tag display (show/hide tags like `[channel:]` and timestamps)
- `F8` - Toggle action pattern highlighting (highlight lines matching action patterns without running commands)
- `Ctrl+C` - Press twice within 15 seconds to quit
- `Ctrl+L` - Redraw screen (filters out client-generated output, keeps only MUD server data)
- `Ctrl+R` - Hot reload (same as /reload)
- `/quit` - Exit the client
- `Enter` - Send command (also releases all pending if paused)

**World Settings Popup (when open):**
- `Up/Down/Tab` - Navigate between fields (auto-enters edit mode for text fields)
- `Left/Right` - Navigate between buttons (when on button row); also scrolls long text fields
- `Enter` - Edit text field / Toggle option / Activate button
- `Space` - Toggle boolean / Cycle options
- `Ctrl+S` - Save all settings and close
- `Esc` - Close popup
- Text fields: Just start typing to edit (inline editing with horizontal scrolling)
- Long text fields show `<` and `>` indicators when content extends beyond visible area
- Buttons: Save, Cancel, Delete, Connect
- Popup sizes dynamically based on content

### Commands

- `/help` - Show help popup with commands and controls (scrollable with arrow keys)
- `/connect [<host> <port> [ssl]]` - Connect to MUD server (uses stored settings if no args); shows "Connecting to host:port..." message
- `/disconnect` (or `/dc`) - Disconnect current world and close log file
- `/send [-W] [-w<world>] [-n] <text>` - Send text to world(s)
  - `-w<world>` - Send to specified world (by name)
  - `-W` - Send to all connected worlds
  - `-n` - Send without end-of-line marker (CR/LF)
  - No flags: Send to current world
- `/setup` - Open Global Settings popup (more mode, spell check, pending first, show tags, input height)
- `/web` - Open Web Settings popup (HTTP/HTTPS servers, WebSocket settings, TLS configuration)
- `/world` - Open World Selector popup (list all worlds, filter, connect or edit)
- `/world <name>` - Connect to world if exists (opens editor if no hostname/port configured), otherwise create and open editor
- `/world -e [name]` - Open World Settings editor for current world or specified world (creates if needed)
- `/world -l <name>` - Connect to world without sending auto-login credentials
- `/worlds` (or `/l`) - List connected worlds in table format with columns:
  - **World**: World name (`*` = current)
  - **Unseen**: Count of unseen lines (empty if 0)
  - **LastSend**: Time since last user command sent
  - **LastRecv**: Time since last data received from server
  - **LastNOP**: Time since last NOP keepalive was sent
  - **NextNOP**: Time until next NOP keepalive
- `/reload` - Hot reload: exec new binary while preserving TCP connections
- `/quit` - Exit the client

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
- TLS/SSL connections cannot be preserved (TLS state is in userspace)
- TLS connections will be closed and need manual reconnection after reload
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

- Settings automatically loaded from `~/.mudclient.dat` on startup
- Settings automatically saved when closing settings popup (Save button, Enter, or Ctrl+S)
- File format is INI-like with `[global]` and `[world:name]` sections
- Hot reload state saved temporarily to `~/.mudclient.reload`

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
- Clients receive all scrollback history and settings on connection
- All MUD data is broadcast to authenticated clients in real-time

**Protocol:**
- JSON over WebSocket
- Message types: AuthRequest, AuthResponse, InitialState, ServerData, WorldConnected, WorldDisconnected, WorldSwitched, PromptUpdate, SendCommand, SwitchWorld, ConnectWorld, DisconnectWorld, MarkWorldSeen, UnseenCleared, Ping, Pong
- Password is hashed with SHA-256 before transmission

**Cross-Interface Sync:**
- When any interface (console, web, or GUI) switches to a world, the unseen count is cleared
- `MarkWorldSeen` message sent by web/GUI clients when switching worlds
- `UnseenCleared` message broadcast to all clients to sync activity indicators
- Console broadcasts `UnseenCleared` when user switches worlds via keyboard

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
- ANSI color rendering (including 256-color and true color support)
- Clickable URLs in output (cyan, underlined, opens in new tab)
- More-mode pausing with Tab/Alt+j
- Command history (Ctrl+P/N)
- Multiple world support with world switching
- World selector popup (`/world` command)
- Connected worlds list (`/worlds` or `/l` command) with arrow key navigation
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
  - Toggle Tags - Show/hide MUD tags (same as F2)
  - Toggle Highlight - Highlight lines matching action patterns (same as F8)
- Font size buttons (S/M/L) next to hamburger:
  - **S** (Small, 11px) - Optimized for phone displays
  - **M** (Medium, 14px) - Default size
  - **L** (Large, 18px) - Optimized for tablet displays
- "Clay" title displayed after font buttons
- Active font button highlighted in cyan

**Mobile Web Interface:**
- Toolbar uses `position: fixed` to stay visible during scrolling and keyboard display
- Uses `100dvh` (dynamic viewport height) instead of `100vh` for proper mobile sizing
- Visual Viewport API tracks keyboard appearance and adjusts toolbar position
- `overscroll-behavior: contain` prevents scroll chaining to parent elements
- `-webkit-overflow-scrolling: touch` for smooth iOS scrolling
- `interactive-widget=resizes-content` viewport meta for proper keyboard handling
- Hamburger icon uses inline SVG for reliable cross-browser rendering

**Mobile Toolbar Layout:**
- Left side: Menu (hamburger), PgUp, PgDn, Tags (👁)
- Right side: ▲ (Previous World), ▼ (Next World)
- PgUp/PgDn on left for easy scrolling access; world switching on right

**Web Interface Controls:**
- `Up/Down` - Switch between active worlds
- `PageUp/PageDown` - Scroll output history
- `Tab` - Release one screenful when paused; scroll down one screenful otherwise (like `more`)
- `Alt+j` - Jump to end, release all pending
- `Ctrl+P/N` - Command history navigation
- `Ctrl+U` - Clear input
- `Ctrl+W` - Delete word before cursor
- `Ctrl+A` - Move cursor to beginning of line
- `Ctrl+Up/Down` - Resize input area
- `F2` - Toggle MUD tag display (show/hide tags and timestamps)
- `F4` - Open filter popup to search output
- `F8` - Toggle action pattern highlighting
- `Enter` - Send command

**Web Popup Controls:**
- `/world` - World selector with filter, arrow keys to navigate, Enter to switch
- `/worlds` or `/l` - Connected worlds list with arrow keys to navigate, Enter to switch
- `/actions` - Actions editor
- `Escape` - Close any popup

**Files:**
- `src/web/index.html` - HTML template
- `src/web/style.css` - CSS styles
- `src/web/app.js` - JavaScript client

### Remote GUI Client

A graphical client mode that connects to a running MudClient's WebSocket server.

**Building:**
```bash
cargo build --features remote-gui    # Requires X11 or Wayland
```

**Running:**
```bash
./mudclient --remote=hostname:port
```

**Features:**
- Graphical window using egui
- Password prompt on startup
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

**Limitations:**
- The remote-gui feature cannot be built in headless environments
- Requires X11 or Wayland display server

### GUI Keyboard Shortcuts

The remote GUI client supports keyboard shortcuts similar to the console client:

**World Switching:**
- `Up/Down` - Cycle through active worlds (when input is empty)
- `Shift+Up/Down` - Cycle through all worlds

**Input Area:**
- `Ctrl+U` - Clear input buffer
- `Ctrl+W` - Delete word before cursor
- `Ctrl+A` - Move cursor to beginning of line
- `Ctrl+P/N` - Previous/Next command history
- `Ctrl+Up/Down` - Resize input area (1-15 lines)
- `Enter` - Send command

**Output Scrollback:**
- `PageUp/PageDown` - Scroll output up/down

**Display:**
- `F2` - Toggle MUD tag display (show/hide `[channel:]` tags and timestamps)
- `F4` - Open filter popup to search output
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

### World Selector Popup

Opened with `/world` command (no arguments):

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

### World Settings Popup Fields

Opened with `/world -e` command:

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
- World Switching - World cycling behavior (Unseen First, Alphabetical)
- Show tags - Show/hide MUD tags at start of lines (default: hidden)
- Input height - Default input area height (1-15 lines)
- Console Theme - Theme for console interface (dark, light)
- GUI Theme - Theme for remote GUI client (dark, light)

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
- Works in console, GUI, and web interfaces
- Setting persists across sessions in `~/.mudclient.dat`

### Actions

Actions are automated triggers that match incoming MUD output against regex patterns and execute commands.

**Action Processing:**
- Incoming lines are checked against all action patterns as they arrive
- Pattern matching is case-insensitive
- ANSI color codes are stripped before pattern matching
- World-specific actions only match for their configured world (empty = all worlds)
- When a pattern matches, the action's commands are executed (sent to the MUD)

**Commands:**
- Multiple commands can be separated by semicolons
- Commands are sent to the MUD server, not processed as local commands
- Special command `/gag` hides the matched line from display (but stores it for F2 viewing)

**Gagging:**
- If an action's command list includes `/gag`, the matched line is hidden
- Gagged lines are stored with a flag and only shown when F2 (show_tags) is enabled
- This allows filtering spam while preserving the ability to review filtered content

**F8 Highlighting:**
- Press F8 to toggle highlighting of lines that match any action pattern
- Matched lines get a dark background color
- Useful for debugging action patterns without running commands

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
