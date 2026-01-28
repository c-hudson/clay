# Clay MUD Client

A terminal-based MUD (Multi-User Dungeon) client built with Rust featuring multi-world support, ANSI color rendering, and a web interface.

## Features

- **Multi-World Support** - Connect to multiple MUD servers simultaneously
- **SSL/TLS** - Secure connections with full TLS support
- **ANSI Colors** - Full ANSI color and formatting support
- **Web Interface** - Browser-based client via WebSocket
- **Remote GUI** - Optional graphical client using egui
- **Remote Console** - Connect to a running Clay instance from another terminal
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
- **Actions/Triggers** - Pattern matching with regex or wildcard, auto-commands
- **TinyFugue Compatibility** - `#` commands for TF users (`#def`, `#set`, `#if`, etc.)
- **Termux Support** - Runs on Android via Termux
- **Android Notifications** - Push notifications via companion app

## Installation

### Pre-built Binaries

Download pre-built binaries from the [Releases](https://github.com/c-hudson/clay/releases) page:

| Platform | Binary | Notes |
|----------|--------|-------|
| Linux x86_64 | `clay-linux-x86_64` | GUI + audio, requires glibc |
| Linux x86_64 (static) | `clay-linux-musl-x86_64` | TUI only, works on any Linux |
| Windows x86_64 | `clay-windows-x86_64.exe` | GUI + audio |
| Android/Termux | `clay-termux-aarch64` | TUI only, ARM64 |

```bash
# Example: Download and install on Linux
curl -L https://github.com/c-hudson/clay/releases/latest/download/clay-linux-x86_64 -o clay
chmod +x clay
./clay
```

## Building from Source

### Linux

```bash
# Install dependencies (Debian/Ubuntu)
sudo apt install libasound2-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxcb1-dev pkg-config

# Easy build with included script (GUI + audio)
./build.sh

# Or manual build with GUI + audio
cargo build --release --features remote-gui-audio

# Static binary for any Linux (TUI only, no GUI)
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl --no-default-features --features rustls-backend
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

### Termux (Android)

```bash
# Install Rust in Termux
pkg install rust

# Build TUI only (no GUI on Android)
cargo build --release --no-default-features --features rustls-backend
```

### Docker

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
./clay

# Run as remote GUI client (connects to running Clay instance)
./clay --remote=hostname:port

# Run as remote console client
./clay --console=hostname:port
```

## Commands

| Command | Description |
|---------|-------------|
| `/worlds` | Open world selector popup |
| `/worlds <name>` | Connect to or create a world |
| `/worlds -e [name]` | Edit world settings |
| `/connections` or `/l` | List connected worlds |
| `/disconnect` or `/dc` | Disconnect current world |
| `/setup` | Open global settings |
| `/web` | Open web/WebSocket settings |
| `/actions` | Open actions/triggers editor |
| `/reload` | Hot reload the binary |
| `/testmusic` | Play test ANSI music sequence |
| `/quit` | Exit the client |
| `/help` | Show help |

## TinyFugue Commands

Clay includes TinyFugue compatibility using `#` prefix:

| Command | Description |
|---------|-------------|
| `#set name value` | Set a variable |
| `#echo message` | Display local message |
| `#def name = body` | Define a macro/trigger |
| `#if (expr) cmd` | Conditional execution |
| `#while (expr) ... #done` | While loop |
| `#for var start end ... #done` | For loop |
| `#bind key = cmd` | Bind key to command |
| `#help [topic]` | Show TF help |

See `#help` for full command list.

## Controls

| Key | Action |
|-----|--------|
| `Up/Down` | Switch between active worlds |
| `PageUp/PageDown` | Scroll output history |
| `Tab` | Release one screenful (when paused) |
| `Escape j` | Jump to end, release all pending |
| `Escape w` | Switch to world with activity |
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

Enable in `/web` settings:

1. Set `HTTP enabled` to Yes (default port: 9000)
2. Set `WS enabled` to Yes for secure WebSocket (default port: 9002)
3. Set a `WS password` (required for authentication)
4. Optionally configure TLS cert/key for HTTPS

Access via browser at `http://localhost:9000`.

## Actions/Triggers

Actions match incoming MUD output against patterns and execute commands:

1. Open `/actions` to create triggers
2. Set a pattern (regex or wildcard)
3. Set command(s) to execute when matched
4. Use `$1`-`$9` for captured groups, `$0` for full match

Example: Pattern `* tells you: *` with command `#echo Got tell from $1`

## Configuration

Settings are stored in `~/.clay.dat`. Per-world settings include:

- Hostname, port, SSL toggle
- Username/password for auto-login
- Character encoding (UTF-8, Latin1, FANSI)
- Auto-login type (Connect, Prompt, MOO_prompt)
- Keepalive type (NOP, Custom, Generic)
- Log file path

## License

MIT
