# Networking Reference

## SSL/TLS Support

- Uses `tokio-native-tls` or `tokio-rustls` for TLS connections (feature-gated)
- Enable via "Use SSL" toggle in world settings
- `StreamReader`/`StreamWriter` enums wrap both plain TCP and TLS streams
- Implements `AsyncRead`/`AsyncWrite` traits for unified handling

## TLS Proxy

When enabled, TLS connections use a proxy process to preserve connections across hot reloads.

**Problem:** TLS state (session keys, IVs, sequence numbers) exists only in process memory. When `exec()` replaces the process, this state is lost.

**Solution:** Forked child process handles TLS, communicates via Unix socket:
```
MUD Server (TLS) <--TLS--> TLS Proxy (child) <--Unix Socket--> Main Client
```

- Toggle in Global Settings (`/setup`), default: disabled
- Socket path: `/tmp/clay-tls-<pid>-<world_name>.sock`
- Proxy PID and socket path saved/restored during reload
- Falls back to direct TLS if proxy spawn fails
- Implementation: `spawn_tls_proxy()`, `run_tls_proxy()`, `StreamReader::Proxy`/`StreamWriter::Proxy`

## Telnet Protocol

### Supported Options

| Option | Code | Description |
|--------|------|-------------|
| SGA | 3 | Accepts server's WILL SGA with DO SGA |
| TTYPE | 24 | Reports terminal type from TERM env var (default: "ANSI") |
| EOR | 25 | Alternative prompt marker, treated same as GA |
| NAWS | 31 | Reports minimum output dimensions across all connected clients |

### NAWS Behavior
- Sends smallest width × height across console + all web/GUI clients
- Updates sent on terminal resize or client dimension change
- Dimensions tracked per-world, reset on disconnect

### Prompt Detection (GA/EOR)
- When telnet GA or EOR is received, text from last newline identified as prompt
- Stored per-world, displayed at start of input area (cyan)
- NOT shown in output area
- Cleared when user sends a command
- Trailing spaces normalized: stripped then one space added

### Keepalive
- Sent every 5 minutes if telnet mode and no data sent
- **NOP** (default): Telnet NOP command (IAC NOP)
- **Custom**: User-defined command
- **Generic**: `help commands ##_idler_message_<rand>_###`

## WebSocket Server

### Configuration (in /web)
- `WS enabled` / `WS port` (default: 9002) / `WS password` (encrypted storage)
- Listens on 0.0.0.0, requires password authentication
- Multiple simultaneous clients supported
- Keepalive: clients send Ping every 30s, server responds with Pong

### Protocol
JSON over WebSocket. Message types:
- **Auth:** AuthRequest, AuthResponse
- **State:** InitialState, ServerData, PromptUpdate
- **World:** WorldConnected, WorldDisconnected, WorldSwitched, ConnectWorld, DisconnectWorld
- **Activity:** UnseenCleared, UnseenUpdate, ActivityUpdate, MarkWorldSeen
- **Commands:** SendCommand, SwitchWorld
- **Health:** Ping, Pong

Key behaviors:
- Password hashed with SHA-256 before transmission
- `InitialState` includes `output_lines` only (not `pending_lines`)
- `pending_count` field shows pending lines; release via PgDn/Tab
- `ServerData` `flush` flag: client clears output buffer before appending (splash screen replacement)
- Server tracks each client's viewed world via `WsClient::current_world`
- `broadcast_to_world_viewers()` routes output only to clients viewing that world

### Allow List / Whitelisting
- `WS allow list` - CSV of IPs that can be whitelisted
- Empty list = always require password
- First auth from allow-list IP whitelists it for future auto-auth
- Only ONE whitelisted IP at a time (new auth clears old)
- Non-allow-list IPs can auth but are never whitelisted

## HTTP/HTTPS Web Interface

- HTTP (default port 9000) uses ws://, HTTPS (default port 9001) uses wss://
- HTTP auto-starts non-secure WebSocket server if not running
- Reuses TLS cert/key from WebSocket settings for HTTPS

### Web Interface Features
- ANSI color rendering (Xubuntu Dark palette, 256-color and true color)
- Shade character blending (░▒▓ as solid blocks with blended colors)
- Clickable URLs (cyan, underlined, opens in new tab)
- More-mode pausing, command history, world switching
- World selector, connections list, actions popups
- Independent view from console (world switching is local)
- Toolbar with hamburger menu and font size controls

### Mobile Web Interface
- `position: fixed` toolbar, `100dvh` for proper mobile sizing
- Visual Viewport API tracks keyboard, `overscroll-behavior: contain`
- Wake-from-background: Ping on visibility change, auto-reconnect if stale

### Mobile Toolbar Layout
- Left: Menu (hamburger), PgUp, PgDn
- Right: ▲ (Previous World), ▼ (Next World)

## WebView GUI Client

```bash
cargo build --features webview-gui
./clay --gui                    # Standalone mode
./clay --gui=hostname:port      # Connect to remote Clay
```

- Native WebView window (wry/tao), same web interface as browser
- ANSI music via rodio
- Hot reload: `GUI_RELOAD_REQUESTED` AtomicBool (WebKit overrides SIGUSR1)
- Splash screen shows clay2.png instead of text ASCII art

## Remote Console Client

```bash
./clay --console=hostname:port
```

- Full terminal interface connecting via WebSocket to master Clay
- All popups work, output scrollback, more-mode, world switching
- World switching is console-local
- No special build features required

## Grep Client

```bash
CLAY_PASSWORD=pass ./clay --grep=hostname:port '*tells you*'        # Search history
CLAY_PASSWORD=pass ./clay --grep=hostname:port -f '*tells you*'     # Follow mode
CLAY_PASSWORD=pass ./clay --grep=hostname:port --regexp -w MyWorld --noesc 'pattern'
```

- `-w <world>` - Limit to specific world
- `--regexp` - Use regex (default: glob)
- `--noesc` - Strip ANSI codes
- `-f` - Follow mode (like tail -f | grep)
- Output: `HH:MM:SS:WorldName  line_text`
- Exit code: 0 if matches, 1 if none

## Hot Reload Details

1. Saves complete state (output buffers, pending lines, scroll positions, per-world settings, showing_splash, etc.)
2. Clears FD_CLOEXEC on socket FDs
3. Calls exec() to replace process with fresh binary
4. New process detects reload mode and restores state
5. TCP sockets reconstructed from preserved FDs
6. Cleanup: disconnected worlds without working command channel fixed, pending lines cleared for disconnected worlds
7. Restored connections have auto-login disabled

**Linux binary update:** When rebuilt while running, `/proc/self/exe` shows " (deleted)" suffix. Reload strips this to find new binary.

**Triggering:** `/reload`, `Ctrl+R`, `SIGUSR1`, or GUI IPC "reload" message.

**Message suppression:** During reload, success messages suppressed (only failures shown).

## Android Notifications

- `/notify <message>` sends notification to Android app
- Works from input or action commands
- Foreground service keeps WebSocket alive in background
- Tapping notification opens app
