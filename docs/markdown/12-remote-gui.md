# Remote GUI Client

Clay includes a native graphical client built with egui that connects to a running Clay instance via WebSocket.

## Building

The remote GUI requires display libraries and is built with the `remote-gui` feature:

```bash
# Linux (requires X11 or Wayland dev libraries)
sudo apt install libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev
cargo build --features remote-gui

# With audio support (for ANSI music)
sudo apt install libasound2-dev
cargo build --features remote-gui-audio

# macOS (no extra dependencies needed)
cargo build --features remote-gui
```

**Note:** The remote-gui feature cannot be built in headless environments.

## Running

```bash
./clay --remote=hostname:port
```

Examples:
```bash
./clay --remote=localhost:9002      # Local secure WebSocket
./clay --remote=mud.server.com:9002 # Remote server
```

## Login Screen

On launch, you'll see the login screen with the Clay logo:

- Enter the WebSocket password
- Click "Connect" or press Enter
- If whitelisted (see Web Interface chapter), auto-connects

## Interface Overview

### World Tabs

Tabs at the top show all worlds:
- ● Connected (filled circle)
- ○ Disconnected (empty circle)
- Click tab to switch worlds

### Output Area

The main area displays MUD output:
- Full ANSI color support
- Scrollable with mouse wheel or PageUp/PageDown
- Clickable URLs (underlined, opens browser)

### Input Field

At the bottom:
- Server prompt displayed in input area
- Type commands and press Enter
- Supports multi-line input

### Status Bar

Shows connection state and current world info.

### Hamburger Menu

Click the menu icon for:

| Option | Description |
|--------|-------------|
| Worlds List | Show connected worlds (Ctrl+L) |
| World Selector | Open world selector popup |
| World Editor | Edit current world (Ctrl+E) |
| Setup | Open settings (Ctrl+S) |
| Font | Adjust font size |
| Toggle Tags | Show/hide MUD tags (F2) |
| Toggle Highlight | Action pattern highlighting (F8) |
| Resync | Request full state refresh |

## Keyboard Shortcuts

### World Switching

| Keys | Action |
|------|--------|
| `Up/Down` | Cycle through active worlds |
| `Shift+Up/Down` | Cycle through all worlds |

### Input Area

| Keys | Action |
|------|--------|
| `Ctrl+U` | Clear input |
| `Ctrl+W` | Delete word |
| `Ctrl+A` | Move to start |
| `Ctrl+P/N` | Command history |
| `Ctrl+Up/Down` | Move cursor in multi-line input |
| `Alt+Up/Down` | Resize input area |
| `Enter` | Send command |

### Output

| Keys | Action |
|------|--------|
| `PageUp/PageDown` | Scroll output |

### Display

| Keys | Action |
|------|--------|
| `F2` | Toggle MUD tags |
| `F4` | Open filter popup |
| `F8` | Toggle action highlighting |
| `Esc` | Close filter popup |

### Menu Shortcuts

| Keys | Action |
|------|--------|
| `Ctrl+L` | Open World List |
| `Ctrl+E` | Edit current world |
| `Ctrl+S` | Open Setup |
| `Ctrl+O` | Connect current world |
| `Ctrl+D` | Disconnect current world |

## Filter Popup

Press `F4` to open the filter popup:

- Type text to filter output (case-insensitive)
- Only matching lines are shown
- ANSI codes stripped for matching, preserved in display
- `Esc` or `F4` to close

## Debug Selection

For troubleshooting ANSI color issues:

1. Highlight text in the output area
2. Right-click to open context menu
3. Select "Debug Selection"
4. A popup shows the raw text with escape codes visible
5. ESC character shown as `<esc>`
6. Copy button available

## Features

### Color Support

- Full 256-color palette
- 24-bit true color
- Xubuntu Dark color scheme

### Colored Square Emoji

Colored square emoji (🟥🟧🟨🟩🟦🟪🟫⬛⬜) are rendered as colored rectangles for consistent appearance.

### Word Wrapping

Long words break at sensible points:
- `[ ] ( ) , \ / - & = ?`
- Preserves URLs and filenames

### ANSI Music

With `remote-gui-audio` feature:
- ANSI music sequences are played
- Uses rodio library
- Requires ALSA on Linux

## Themes

Configure GUI theme in `/setup`:
- **Dark**: Dark background, light text (default)
- **Light**: Light background, dark text

## Synchronization

The GUI stays synchronized with console and web clients:

- All output is shared
- Unseen counts sync
- Activity indicators match
- World switching is GUI-local (doesn't affect other clients)

## Troubleshooting

### Build Errors

**"Could not find X11 libraries"**
```bash
sudo apt install libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev
```

**"Could not find ALSA"** (for audio)
```bash
sudo apt install libasound2-dev
```

### Connection Issues

1. Verify Clay console is running with WebSocket enabled
2. Check the port matches your `/web` configuration
3. Try connecting with the web interface first to verify password

### Display Issues

1. Ensure X11 or Wayland is running
2. Check DISPLAY environment variable
3. Try different theme (dark vs light)

### No Sound

1. Verify `remote-gui-audio` feature was enabled at build
2. Check ANSI Music is enabled in `/setup`
3. Use `/testmusic` to verify audio works

\newpage

