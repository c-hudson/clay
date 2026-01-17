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
- **TLS Proxy** - Optional proxy to preserve TLS connections across hot reload
- **Spell Check** - Built-in spell checking with suggestions
- **Output Filtering** - Search/filter output with F4
- **File Logging** - Per-world output logging
- **ANSI Music** - BBS-style music playback (web/GUI interfaces)

## Installation

### Pre-built Binaries

Download pre-built binaries from the [Releases](https://github.com/c-hudson/clay/releases) page:

| Platform | Binary | Notes |
|----------|--------|-------|
| Linux x86_64 | `clay-linux-x86_64` | GUI + audio, requires glibc |
| Linux x86_64 (static) | `clay-linux-musl-x86_64` | TUI only, works on any Linux |
| Linux x86_64 (TUI) | `clay-linux-x86_64-tui` | TUI only, smaller binary |
| macOS x86_64 | `clay-macos-x86_64` | Intel Macs |
| macOS ARM | `clay-macos-aarch64` | Apple Silicon (M1/M2/M3) |
| Windows x86_64 | `clay-windows-x86_64.exe` | GUI + audio |

```bash
# Example: Download and install on Linux
curl -L https://github.com/c-hudson/clay/releases/latest/download/clay-linux-x86_64 -o clay
chmod +x clay
./clay
```

## Building from Source

### Linux

Use the included build script to build with GUI and audio support:

```bash
./build.sh
```

This builds a release binary with `--features remote-gui-audio` and prompts for an install location (defaults to `$HOME`).

**System dependencies (Debian/Ubuntu):**
```bash
sudo apt install libasound2-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxcb1-dev pkg-config
```

**Manual build options:**

```bash
# Standard build (TUI only)
cargo build --release

# Static binary (requires musl, TUI only)
cargo build --release --target x86_64-unknown-linux-musl --no-default-features --features rustls-backend

# With remote GUI support (requires X11/Wayland)
cargo build --release --features remote-gui

# With remote GUI and ANSI music audio
cargo build --release --features remote-gui-audio
```

### macOS

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Build with GUI + audio
cargo build --release --features remote-gui-audio
```

### Windows

```bash
# Install Rust from https://rustup.rs

# Build with GUI + audio
cargo build --release --features remote-gui-audio
```

### Docker

Build using Docker for a consistent environment:

```bash
# Build with GUI + audio (glibc)
docker build -t clay-builder .
docker run --rm -v $(pwd)/output:/output clay-builder

# Build static binary (musl, TUI only)
docker build -f Dockerfile.static -t clay-static-builder .
docker run --rm -v $(pwd)/output:/output clay-static-builder

# Binary will be in ./output/clay
```

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

Settings are stored in `~/.clay.dat`. Per-world settings include:

- Hostname, port, SSL toggle
- Username/password for auto-login
- Character encoding (UTF-8, Latin1, FANSI)
- Log file path
- Keepalive type (NOP, GMCP, None)

## License

MIT
