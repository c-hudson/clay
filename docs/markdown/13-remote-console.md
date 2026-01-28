# Remote Console Client

Clay includes a remote console client that provides the full terminal interface while connecting to a master Clay instance via WebSocket.

## Running

```bash
./clay --console=hostname:port
```

Examples:
```bash
./clay --console=localhost:9002         # Local secure WebSocket
./clay --console=mud.server.com:9002    # Remote server
```

No special build features required - works with the standard musl build.

## Use Cases

- Access your MUD sessions from another terminal/SSH
- Run Clay on a server, connect from anywhere
- Have multiple terminal views of the same sessions

## Interface

The remote console provides the identical interface to the main console:

- Full terminal UI with ratatui/crossterm
- All popup dialogs (help, menu, settings, world selector, etc.)
- Output scrollback with PageUp/PageDown
- More-mode pausing
- World switching
- Command history
- Spell checking (if enabled on master)

## Key Differences from Main Console

### No Direct Connections

- All MUD connections go through the master instance
- Commands are forwarded to the master
- Output is received via WebSocket

### Local World Switching

- Switching worlds only affects your view
- Doesn't change what the master or other clients see
- Each remote console can view different worlds

### Synchronized State

- Output history is shared across all clients
- Unseen counts sync when any client views a world
- Settings changes affect all clients

## Keyboard Shortcuts

All standard console shortcuts work:

| Keys | Action |
|------|--------|
| `Up/Down` | Switch active worlds |
| `Shift+Up/Down` | Switch all worlds |
| `PageUp/PageDown` | Scroll output |
| `Tab` | More-mode release / scroll |
| `Escape+j` | Jump to end |
| `Ctrl+P/N` | Command history |
| `Ctrl+U` | Clear input |
| `F1` | Help popup |
| `F2` | Toggle MUD tags |
| `F4` | Filter popup |
| `F8` | Action highlighting |
| `Ctrl+L` | Redraw screen |

### Special Commands

| Command | Description |
|---------|-------------|
| `/menu` | Open hamburger menu popup |
| `/version` | Display version information |

## Menu Popup

Access with `/menu` or hamburger icon:

| Option | Description |
|--------|-------------|
| Worlds List | Connected worlds |
| World Selector | All worlds |
| World Editor | Edit current world |
| Setup | Global settings |
| Web | Web settings |
| Actions | Actions editor |
| Toggle Tags | Show/hide MUD tags |
| Toggle Highlight | Action highlighting |
| Resync | Refresh from master |

## Popups

All popup dialogs work in remote console mode:

- **Help** (F1): Scrollable help content
- **Settings** (/setup): Global settings
- **Web Settings** (/web): WebSocket/HTTP configuration
- **World Selector** (/worlds): Browse and switch worlds
- **World Editor** (/worlds -e): Edit world settings
- **Actions** (/actions): Edit triggers
- **Filter** (F4): Search output

Popups use the unified popup system with consistent controls.

## Authentication

Uses the same authentication as web interface:

1. Connect to master's WebSocket
2. Enter password (or auto-connect if whitelisted)
3. Receive full state sync
4. Begin using the client

## Building

No special features needed:

```bash
# Standard musl build works
cargo build --target x86_64-unknown-linux-musl \
    --no-default-features --features rustls-backend
```

## Example Setup

### Server Side (Master)

```bash
# Start Clay normally
./clay

# Enable WebSocket in /web settings:
# - WS enabled: On
# - WS port: 9002
# - WS password: your_password
```

### Client Side (Remote)

```bash
# Connect from anywhere
./clay --console=your-server.com:9002

# Enter password when prompted
```

## Tips

### SSH Forwarding

For secure access without TLS:

```bash
# On client machine
ssh -L 9002:localhost:9002 your-server

# Then connect locally
./clay --console=localhost:9002
```

### Multiple Views

Run multiple remote consoles to:
- Monitor different worlds simultaneously
- Have different scroll positions
- Use different tag visibility settings

### Screen/Tmux

Combine with screen or tmux for persistent sessions:

```bash
# On server
tmux new -s clay
./clay

# From anywhere
ssh your-server -t tmux attach -t clay
```

## Troubleshooting

### Connection Refused

1. Verify master Clay is running
2. Check WebSocket is enabled in `/web`
3. Verify port matches

### Authentication Failed

1. Check password is correct
2. Verify allow list configuration if using whitelisting

### Display Issues

1. Check TERM environment variable
2. Try `Ctrl+L` to redraw
3. Verify terminal supports colors

### Sync Issues

1. Use `/menu` → Resync to refresh state
2. Check network connection
3. Restart remote console

\newpage

