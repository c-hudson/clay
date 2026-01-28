# Keyboard Shortcuts

## World Switching

| Keys | Action |
|------|--------|
| `Up` / `Down` | Cycle through active worlds (connected OR with unseen output) |
| `Shift+Up` / `Shift+Down` | Cycle through all worlds (including disconnected) |
| `Escape` then `w` | Switch to world with activity |
| `Alt+w` | Switch to world with activity (same as Escape+w) |

**Activity priority:** Oldest pending lines → Unseen output → Previous world

## Input Area

| Keys | Action |
|------|--------|
| `Left` / `Right` | Move cursor |
| `Ctrl+B` / `Ctrl+F` | Move cursor (alternative) |
| `Ctrl+Up` / `Ctrl+Down` | Move cursor up/down in multi-line input |
| `Alt+Up` / `Alt+Down` | Resize input area (1-15 lines) |
| `Ctrl+U` | Clear entire input |
| `Ctrl+W` | Delete word before cursor |
| `Ctrl+A` | Jump to start of line |
| `Home` | Jump to start of line |
| `End` | Jump to end of line |
| `Ctrl+P` | Previous command in history |
| `Ctrl+N` | Next command in history |
| `Ctrl+Q` | Spell suggestions / cycle and replace |
| `Tab` | Command completion (when input starts with `/` or `#`) |
| `Enter` | Send command |

## Output Scrollback

| Keys | Action |
|------|--------|
| `PageUp` | Scroll back in history (enables more-pause) |
| `PageDown` | Scroll forward (unpauses if at bottom) |
| `Tab` | Release one screenful when paused; scroll down when viewing history |
| `Escape` then `j` | Jump to end, release all pending lines |

## More-Mode (When Paused)

| Keys | Action |
|------|--------|
| `Tab` | Release one screenful of pending lines |
| `PageDown` | Release all pending and scroll to bottom |
| `Escape` then `j` | Jump to end, release all pending |
| `Enter` | Send command (does NOT release pending) |

## General

| Keys | Action |
|------|--------|
| `F1` | Open help popup |
| `F2` | Toggle MUD tag display (show/hide channel tags and timestamps) |
| `F4` | Open filter popup to search output |
| `F8` | Toggle action pattern highlighting |
| `Ctrl+C` | Press twice within 15 seconds to quit |
| `Ctrl+L` | Redraw screen (filters out client-generated output) |
| `Ctrl+R` | Hot reload (same as /reload) |
| `Ctrl+Z` | Suspend process (use `fg` to resume) |

## Popup Controls (All Popups)

| Keys | Action |
|------|--------|
| `Up` / `Down` | Navigate between fields |
| `Tab` / `Shift+Tab` | Cycle through buttons only |
| `Left` / `Right` | Navigate between buttons; change select/toggle values |
| `Enter` | Edit text field / Toggle option / Activate button |
| `Space` | Toggle boolean / Cycle options |
| `Esc` | Close popup or cancel text edit |

**Button shortcuts:** Letters are highlighted in button labels (e.g., **S**ave, **C**ancel, **D**elete)

## Filter Popup (F4)

| Keys | Action |
|------|--------|
| Type text | Filter output to matching lines |
| `Backspace` / `Delete` | Edit filter text |
| `Left` / `Right` | Move cursor in filter text |
| `Home` / `End` | Jump to start/end of filter |
| `PageUp` / `PageDown` | Scroll through filtered results |
| `Esc` / `F4` | Close filter and restore normal view |

## Help Popup (F1)

| Keys | Action |
|------|--------|
| `Up` / `Down` | Scroll one line |
| `PageUp` / `PageDown` | Scroll multiple lines |
| `O` | Highlight Ok button |
| `Enter` / `Esc` | Close popup |

## World Selector (/worlds)

| Keys | Action |
|------|--------|
| `Up` / `Down` | Navigate world list |
| `Tab` / `Shift+Tab` | Cycle between list and buttons |
| `Enter` | Connect to selected world / Activate button |
| `Left` / `Right` | Move between buttons |
| `A` | Add new world |
| `E` | Edit selected world |
| `/` | Focus filter box |
| `Esc` | Close popup |

## Actions List (/actions)

| Keys | Action |
|------|--------|
| `Up` / `Down` | Navigate action list |
| `Space` | Toggle enable/disable selected action |
| `Enter` | Edit selected action |
| `Tab` | Cycle between list and buttons |
| `A` | Add new action |
| `E` | Edit selected action |
| `D` | Delete selected action |
| `C` | Cancel/close |
| `F` or `/` | Focus filter box |
| `Esc` | Close popup |

## Confirmation Dialogs

| Keys | Action |
|------|--------|
| `Left` / `Right` / `Up` / `Down` / `Tab` | Toggle between Yes/No |
| `Y` | Select Yes |
| `N` | Select No |
| `Enter` | Confirm selection |
| `Esc` | Cancel and close |

## Remote GUI Client Additional Shortcuts

| Keys | Action |
|------|--------|
| `Ctrl+L` | Open World List popup |
| `Ctrl+E` | Open World Editor for current world |
| `Ctrl+S` | Open Setup popup |
| `Ctrl+O` | Connect current world |
| `Ctrl+D` | Disconnect current world |

\newpage

