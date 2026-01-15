# Clay MUD Client

A terminal-based MUD (Multi-User Dungeon) client built with Rust featuring multi-world support, ANSI color rendering, and a web interface.

## Features

- **Multi-World Support** - Connect to multiple MUD servers simultaneously
- **SSL/TLS** - Secure connections with full TLS support
- **ANSI Colors** - Full ANSI color and formatting support
- **Web Interface** - Browser-based client via WebSocket
- **Remote GUI** - Optional graphical client using egui
- **More-Mode** - Pagination for fast-scrolling output
- **Scrollback** - Unlimited history with PageUp/PageDown navigation
- **Command History** - Navigate previous commands with Ctrl+P/N
- **Telnet Protocol** - Full telnet negotiation with keepalive support
- **Auto-Login** - Configurable automatic login (Connect, Prompt, MOO modes)
- **Hot Reload** - Update the binary without losing connections (`/reload`)
- **Spell Check** - Built-in spell checking with suggestions
- **Output Filtering** - Search/filter output with F4
- **File Logging** - Per-world output logging
- **ANSI Music** - BBS-style music playback (web/GUI interfaces)

## Building

### Linux

Use the included build script to build with GUI and audio support:

```bash
./build.sh
```

This builds a release binary with `--features remote-gui-audio` and prompts for an install location (defaults to `$HOME`).

**Manual build options:**

```bash
# Standard build
cargo build --release

# Static binary (requires musl)
CC=musl-gcc cargo build --release --target x86_64-unknown-linux-musl --no-default-features --features rustls-backend

# With remote GUI support (requires X11/Wayland)
cargo build --release --features remote-gui

# With remote GUI and ANSI music audio (requires libasound2-dev)
cargo build --release --features remote-gui-audio
```

### macOS

*To be determined*

### Windows

*To be determined*

## Usage

```bash
# Run the TUI client
./target/release/clay

# Run as remote GUI client
./target/release/clay --remote=hostname:port
```

## Commands

| Command | Description |
|---------|-------------|
| `/connect [host port [ssl]]` | Connect to a MUD server |
| `/disconnect` or `/dc` | Disconnect current world |
| `/world` | Open world selector |
| `/world <name>` | Connect to or create a world |
| `/world -e [name]` | Edit world settings |
| `/worlds` or `/l` | List connected worlds |
| `/setup` | Open global settings |
| `/reload` | Hot reload the binary |
| `/testmusic` | Play test ANSI music sequence |
| `/quit` | Exit the client |
| `/help` | Show help |

## Controls

| Key | Action |
|-----|--------|
| `Up/Down` | Switch between active worlds |
| `PageUp/PageDown` | Scroll output history |
| `Tab` | Release one screenful (when paused) |
| `Alt+j` | Jump to end, release all pending |
| `Ctrl+P/N` | Command history |
| `Ctrl+U` | Clear input |
| `Ctrl+W` | Delete word |
| `Ctrl+Q` | Spell suggestions |
| `Ctrl+L` | Redraw screen |
| `Ctrl+R` | Hot reload |
| `F1` | Help |
| `F2` | Toggle MUD tag display |
| `F4` | Filter output |
| `F8` | Toggle action highlighting |
| `Ctrl+C` (x2) | Quit |

## Web Interface

Enable the WebSocket server in `/setup` to access the web interface:

1. Set `WS enabled` to Yes
2. Configure `WS port` (default: 9001)
3. Set a `WS password`
4. Optionally enable HTTPS with cert/key files

Access via browser at `http://localhost:9001` (or `https://` if configured).

## Configuration

Settings are stored in `~/.mudclient.dat`. Per-world settings include:

- Hostname, port, SSL toggle
- Username/password for auto-login
- Character encoding (UTF-8, Latin1, FANSI)
- Log file path
- Keepalive type (NOP, GMCP, None)

## License

MIT
