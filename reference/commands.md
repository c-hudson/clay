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

Clay's default keybindings follow TinyFugue 5.0's own keymap; `^Q` and `^R`
are the only two Clay keeps at their historical (non-TF) meaning, plus a
handful of Clay-only extras (the F-keys, `^Y`, `Shift-Up/Down`, `Alt-Up/Down`).
The full, generated-from-code table (every default, every chord, the numeric
prefix, insert mode) lives in `docs/markdown/07-keyboard-shortcuts.md` and is
kept honest by `cargo test test_docs_key_table_matches_defaults`
(`src/keybindings.rs`). This section is a summary; see that file for
anything not covered here, and `TINYFUGUE-COMPAT.md` for which defaults
changed from Clay's pre-parity keymap and why.

The web and GUI clients dispatch keys through the *same* four-level order as
the console (see "Configurable Keybindings" below) and share the same
default table — a TF `/bind` entry set on the server reaches them too via
`RunKeyBinding` (see below). Known exceptions: `expand_line` (`Esc-^E`) is a
console/GUI-local no-op in the plain browser client (no safe wire path to
substitute a remote client's own input line), and `world_socket_prev/next`
reuse the browser's existing "switch to a known world index" helper rather
than a dedicated wire message.

### World Switching
- `Esc-Left`/`Esc-Right` - Cycle *connected* worlds only (TF SOCKETB/SOCKETF)
- `Esc-{`/`Esc-}` - Cycle *active* worlds (`world_prev`/`world_next`, unseen output first)
- `Shift+Up/Down` - Cycle through all worlds (including disconnected)
- `Escape` then `w` - Switch to world with activity (priority: oldest pending → unseen output → previous world)
- `^]` - Background all worlds (TF `/bg`); a no-op in Clay's single-pane console

Clay's own "unseen-first" active-world cycling (action ids `world_next`/
`world_prev`, governed by the "World Switching" setting in `/setup` -
Unseen First vs. Alphabetical) keeps `Esc-{`/`Esc-}`: TF binds those to socket
cycling too, so Clay spends the redundant pair on its own cycling instead.

### Input Area
- `Left/Right` or `Ctrl+B/Ctrl+F` - Move cursor one character
- `Escape` then `b/f`, or `Ctrl+Left/Right` - Move cursor one word left/right (TF: wleft/wright)
- `Up/Down` - Move cursor up/down in multi-line input (TF default)
- `Ctrl+A` or `Home` - Jump to start of line
- `Ctrl+E` or `End` - Jump to end of line
- `Ctrl+U` - Kill to start of line (TF's real `^U`; kill ring kept - Clay's older "clear whole line" meaning is the separate `clear_line` action, unbound by default)
- `Ctrl+W` - Delete word backward (space-delimited)
- `Ctrl+K` - Kill to end of line (pushes to kill ring)
- `Ctrl+D` - Delete character forward
- `Ctrl+Y` - Yank (paste from kill ring)
- `Ctrl+T` - Transpose two characters before cursor
- `Ctrl+V` - Insert next character literally (console only, not web)
- `Ctrl+P/N` or `Ctrl+Up/Down` - Previous/Next command history
- `Ctrl+Home/End` or `Esc-<`/`Esc->` - Jump to oldest/newest history entry (TF RECALLBEG/RECALLEND)
- `Insert` or `Escape v` - Toggle insert/overwrite mode
- `Escape 0`-`9` / `Escape -` - Build a numeric repeat-count prefix (`%kbnum`; shown as `[N]` in the separator bar, cleared by the next non-digit action or `^G`)
- `Ctrl+Q` - Spell suggestions / cycle and replace
- `Ctrl+G` - Terminal bell/beep; also cancels a buffered chord or numeric prefix
- `Tab` / `Escape Tab` - Release pending output (more-mode priority) / command completion
- `Escape` then `c/l/u` - Capitalize / lowercase / uppercase word
- `Escape` then `d` - Delete word forward (pushes to kill ring)
- `Escape` then `Space` - Collapse multiple spaces around cursor to one
- `Escape` then `=` - Jump to matching bracket (`()[]{}`)
- `Escape` then `.` or `_` - Insert last word from previous history entry
- `Escape` then `p` - Search history backward (entries starting with current input)
- `Escape` then `n` - Search history forward (continues backward search)
- `Escape` then `Backspace` (or `Esc-^H`, `^X^?`) - Delete word backward (punctuation-delimited, pushes to kill ring)
- `Alt+Up/Down` - Resize input area (1-15 lines)

**Kill Ring:** `Ctrl+K`, `Ctrl+U`, `Ctrl+W`, `Escape+d`, and `Escape+Backspace` push deleted text to the kill ring. `Ctrl+Y` pastes the most recent entry.

### Output Scrollback
- `PageUp` - Scroll back in history (enables more-pause)
- `PageDown` or `Ctrl+PageDown` - Scroll forward / release all pending (unpauses if at bottom)
- `Tab` - Release one screenful of pending lines (when paused); command completion when input starts with `/`; otherwise pages like PgDn
- `Escape` then `j` - Jump to end, release all pending lines
- `Escape` then `J` (uppercase) - Selective flush: keep only highlighted pending lines, discard rest
- `Escape` then `h` - Half-page scroll up or release half screenful of pending
- `Escape` then `^N`/`^P` - Scroll one line forward/back (TF LINE/LINEBACK)
- `Escape` then `^L` - Clear the view without dropping scrollback (TF CLEAR)
- `^S` - Pause the current world (TF PAUSE)
- `Escape L` - Toggle the F4 filter popup's last-applied limit on/off
- `^X[`/`^X]`/`^X{`/`^X}` - Half-page back/forward, page back/forward (TF's own `^X` prefix map)
- `F4` - Open filter popup to search output

### General
- `F1` - Open help popup
- `F2` - Toggle MUD tag display (show/hide tags like `[channel:]` and timestamps)
- `F5` - Search command history
- `F8` - Toggle action pattern highlighting (highlight lines matching action patterns without running commands)
- `F9` - Toggle GMCP media audio (master mute switch, starts muted)
- `Ctrl+C` - Press twice within 15 seconds to quit
- `Ctrl+L` - Refresh screen (plain repaint, TF REFRESH). Clay's older "repaint and drop client-generated lines" meaning is the separate `redraw_server_only` action, unbound by default.
- `Ctrl+R` or `^X^R` - Hot reload (same as /reload)
- `^X^V` - Show version (same as /version)
- `Ctrl+Z` - Suspend process (use `fg` to resume)
- `Enter` - Send command

### Popup Controls (unified popup system)
- `Up/Down` - Navigate between fields (auto-enters edit mode for text fields)
- `Tab/Shift+Tab` - Cycle through buttons only
- `Left/Right` - Navigate between buttons (when on button row); change select/toggle values
- `Enter` - Edit text field / Toggle option / Activate button
- `Space` - Toggle boolean / Cycle options
- `Esc` - Close popup or cancel text edit
- Buttons have highlighted shortcut letters (the actual letter varies by popup and button - e.g. the World Selector's Add/Edit/Delete/Cancel/Connect buttons are `A`/`E`/`D`/`C`/`O`)
- Popups size dynamically based on content

### Mouse Controls (when Console Mouse enabled in /setup, default: on)
- Left click on popup buttons to activate them
- Left click on popup fields to select and edit/toggle them
- Left click on list items to select them
- Scroll wheel up/down to scroll list items and scrollable content in popups
- Click and drag in scrollable content or list fields to highlight lines of text
- Any keyboard input clears the highlight

## Configurable Keybindings

All non-character keys are configurable via `~/.clay/keybindings.dat` (INI
format; the legacy `~/.clay.key.dat` path is no longer used - old installs
migrate automatically). Defaults follow TinyFugue conventions (see above).

**Dispatch order**, checked for every keypress, console and web/GUI alike:
1. A TF `/bind`/`/def -b`/`/def -B` binding for the exact key (or chord) pressed - runtime, set via `/bind` and stored in the TF engine, not `keybindings.dat`. On a remote (web/GUI/SSH-console) client, the server tells the client which keys it has TF bindings for (`tf_bound_keys_json`/`GlobalSettingsMsg`) and the client sends a matched keypress back as `WsMessage::RunKeyBinding{key, kbnum}` for the server to actually run.
2. A `key_<name>` macro (TF's own two-level naming for physical keys - `/def key_f5 = ...`, `/def key_esc_left = ...`; see `/help keys`).
3. The built-in action table (`keybindings.dat`, falling back to the compiled-in defaults).
4. Ordinary literal character input (nothing bound - the key is just typed).

**Key name grammar** (`src/keynames.rs`): `^A` (Ctrl+A, upper-cased on
letters), a named key (`Up`, `Down`, `Left`, `Right`, `PageUp`/`PgUp`,
`PageDown`/`PgDn`, `Home`, `End`, `Insert`/`Ins`, `Delete`/`Del`,
`Backspace`/`BS`, `Tab`, `Enter`/`Return`, `Escape`/`Esc`, `Space`,
`F1`-`F20`), a real terminal modifier held with a named key (`Ctrl-Up`,
`Shift-Tab`, `Alt-Down` - one physical keystroke), `Esc-<token>` (Escape then
one more token, case-significant: `Esc-j` != `Esc-J`) - `Alt-x`/`Meta-x`/`@x`
are accepted spellings of the same `Esc-x`, except before a named key, where
they mean the real modifier instead (`Alt-Up` = `Ctrl-Up`'s sibling, not
`Esc-Up`) - or a chord: two or more tokens written back to back with no
separator (`^X^R`, `Esc-^N`). TinyFugue's own raw spellings are also
accepted and normalised: `^[b`, `\033`, `\0x1B`, `\27` (the escape byte), and
raw terminal escape sequences (`^[[1;5A`). See `/help bind`.

**Action IDs:** Each binding maps a key name to an action ID string (e.g. `cursor_home`, `history_prev`, `world_socket_next`). See `keybindings::ACTIONS` for the full list, or the keybind editor's own category list.

**File format (`~/.clay/keybindings.dat`):**
```ini
[bindings]
Up = world_next
Down = world_prev
Ctrl-Up = UNBOUND
```
Only non-default bindings need to be saved. Use `UNBOUND` to remove a default binding.
