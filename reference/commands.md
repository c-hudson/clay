# Commands & Controls Reference

## Client Commands

- `/help` - Show help popup (90% terminal width, scrollable, word-wrapped)
- `/disconnect` (or `/dc`) - Disconnect current world and close log file
- `/send [-W] [-w<world>] [-n] <text>` - Send text to world(s)
  - `-w<world>` - Send to specified world (by name)
  - `-W` - Send to all connected worlds
  - `-n` - Send without end-of-line marker (CR/LF)
- `/setup` - Open Global Settings popup (more mode, spell check, temp convert, world switching, show tags, input height, themes, mouse, ZWJ, ANSI music, TLS proxy)
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

## Keyboard Controls (TF Defaults)

### World Switching
- `Ctrl+Up/Down` - Cycle through active worlds (connected OR with unseen output)
- `Shift+Up/Down` - Cycle through all worlds
- `Escape` then `w` - Switch to world with activity (priority: oldest pending → unseen output → previous world)

World switching behavior is controlled by the "World Switching" setting:
1. **Unseen First**: If any OTHER world has unseen output, switch to the world that received unseen output first (oldest unseen). Done.
2. **Alphabetical** (or when no unseen): Switch to the alphabetically next world by name. Wraps from the last world back to the first.

### Input Area
- `Left/Right` or `Ctrl+B/Ctrl+F` - Move cursor one character
- `Escape` then `b/f` - Move cursor one word left/right (TF: wleft/wright)
- `Up/Down` - Move cursor up/down in multi-line input (TF default)
- `Ctrl+A` or `Home` - Jump to start of line
- `Ctrl+E` or `End` - Jump to end of line
- `Ctrl+U` - Clear line
- `Ctrl+W` - Delete word backward (space-delimited)
- `Ctrl+K` - Kill to end of line (pushes to kill ring)
- `Ctrl+D` - Delete character forward
- `Ctrl+Y` - Yank (paste from kill ring)
- `Ctrl+T` - Transpose two characters before cursor
- `Ctrl+V` - Insert next character literally (console only, not web)
- `Ctrl+P/N` - Previous/Next command history
- `Ctrl+Q` - Spell suggestions / cycle and replace
- `Ctrl+G` - Terminal bell/beep
- `Tab` - Command completion (when input starts with `/` or `#`); more-mode takes priority if paused
- `Escape` then `c/l/u` - Capitalize / lowercase / uppercase word
- `Escape` then `d` - Delete word forward (pushes to kill ring)
- `Escape` then `Space` - Collapse multiple spaces around cursor to one
- `Escape` then `-` - Jump to matching bracket (`()[]{}`)
- `Escape` then `.` or `_` - Insert last word from previous history entry
- `Escape` then `p` - Search history backward (entries starting with current input)
- `Escape` then `n` - Search history forward (continues backward search)
- `Escape` then `Backspace` - Delete word backward (punctuation-delimited, pushes to kill ring)
- `Alt+Up/Down` - Resize input area (1-15 lines)

**Kill Ring:** `Ctrl+K`, `Ctrl+U`, `Ctrl+W`, `Escape+d`, and `Escape+Backspace` push deleted text to the kill ring. `Ctrl+Y` pastes the most recent entry.

### Output Scrollback
- `PageUp` - Scroll back in history (enables more-pause)
- `PageDown` - Scroll forward (unpauses if at bottom)
- `Tab` - Release one screenful of pending lines (when paused); scroll down like PgDn (when viewing history)
- `Escape` then `j` - Jump to end, release all pending lines
- `Escape` then `J` (uppercase) - Selective flush: keep only highlighted pending lines, discard rest
- `Escape` then `h` - Half-page scroll up or release half screenful of pending
- `F4` - Open filter popup to search output

### General
- `F1` - Open help popup
- `F2` - Toggle MUD tag display (show/hide tags like `[channel:]` and timestamps)
- `F8` - Toggle action pattern highlighting (highlight lines matching action patterns without running commands)
- `F9` - Toggle GMCP media audio (master mute switch, starts muted)
- `Ctrl+C` - Press twice within 15 seconds to quit
- `Ctrl+L` - Redraw screen (filters out client-generated output, keeps only MUD server data)
- `Ctrl+R` - Hot reload (same as /reload)
- `Ctrl+Z` - Suspend process (use `fg` to resume)
- `Enter` - Send command

### Popup Controls (unified popup system)
- `Up/Down` - Navigate between fields (auto-enters edit mode for text fields)
- `Tab/Shift+Tab` - Cycle through buttons only
- `Left/Right` - Navigate between buttons (when on button row); change select/toggle values
- `Enter` - Edit text field / Toggle option / Activate button
- `Space` - Toggle boolean / Cycle options
- `S/C/D/O` - Shortcut keys for Save/Cancel/Delete/Connect buttons (when available)
- `Esc` - Close popup or cancel text edit
- Buttons have highlighted shortcut letters
- Popups size dynamically based on content

### Mouse Controls (when Console Mouse enabled in /setup, default: on)
- Left click on popup buttons to activate them
- Left click on popup fields to select and edit/toggle them
- Left click on list items to select them
- Scroll wheel up/down to scroll list items and scrollable content in popups
- Click and drag in scrollable content or list fields to highlight lines of text
- Any keyboard input clears the highlight

## Web Interface Controls

- `Up/Down` - Switch between active worlds
- `PageUp/PageDown` - Scroll output history
- `Tab` - Release one screenful when paused; scroll down one screenful otherwise
- `Escape+j` - Jump to end, release all pending
- `Escape+J` - Selective flush (keep highlighted pending, discard rest)
- `Escape+w` or `Alt+w` - Switch to world with activity (oldest pending/unseen)
- `Escape+b` / `Escape+f` - Move cursor one word left/right
- `Escape+h` - Half-page scroll/release
- `Ctrl+P/N` - Command history navigation
- `Ctrl+U` - Clear input
- `Ctrl+W` - Delete word before cursor
- `Ctrl+A` - Move cursor to beginning of line
- `Ctrl+T` - Transpose characters
- `Alt+Up/Down` - Resize input area
- `Escape+Space` - Collapse multiple spaces to one
- `Escape+-` - Goto matching bracket
- `Escape+.` / `Escape+_` - Insert last word of previous history
- `Escape+p` / `Escape+n` - Search history backward/forward
- `Escape+Backspace` - Delete word back (punctuation-delimited)
- `F2` - Toggle MUD tag display
- `F4` - Open filter popup to search output
- `F8` - Toggle action pattern highlighting
- `F9` - Toggle GMCP media audio
- `Enter` - Send command

## GUI Keyboard Shortcuts

### World Switching
- `Up/Down` - Cycle through active worlds
- `Shift+Up/Down` - Cycle through all worlds

### Menu Shortcuts
- `Ctrl+L` - Open World List popup
- `Ctrl+E` - Open World Editor for current world
- `Ctrl+S` - Open Setup popup
- `Ctrl+O` - Connect current world
- `Ctrl+D` - Disconnect current world

### Other
- `F2` - Toggle MUD tag display
- `F4` - Open filter popup
- `F8` - Toggle action pattern highlighting
- `F9` - Toggle GMCP media audio
- `PageUp/PageDown` - Scroll output
- `Alt+Up/Down` - Resize input area

## Configurable Keybindings

All non-character keys are configurable via `~/.clay.key.dat`. Defaults follow TinyFugue conventions. Two layers checked in order:
1. TF `/bind` bindings (runtime, from `/bind` command)
2. Action bindings (from `~/.clay.key.dat`, falling back to TF defaults)

**Key name format:** `^A` (Ctrl+A), `Esc-x` (Escape then x), `F1`-`F12`, `Up`, `Down`, `Left`, `Right`, `PageUp`, `PageDown`, `Home`, `End`, `Insert`, `Delete`, `Backspace`, `Tab`, `Enter`, `Escape`, `Shift-Up`, `Ctrl-Down`, `Alt-Up`, etc.

**Action IDs:** Each binding maps a key name to an action ID string (e.g. `cursor_home`, `history_prev`, `world_next`). See `keybindings::ACTIONS` for the full list.

**File format (`~/.clay.key.dat`):**
```ini
[bindings]
Up = world_next
Down = world_prev
Ctrl-Up = UNBOUND
```
Only non-default bindings need to be saved. Use `UNBOUND` to remove a default binding.
