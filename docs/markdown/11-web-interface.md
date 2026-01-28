# Web Interface

Clay includes a browser-based client that connects via WebSocket to control your MUD sessions from anywhere.

## Setup

### 1. Configure WebSocket Server

Open `/web` settings:

```
/web
```

Set up either secure (recommended) or non-secure WebSocket:

**Secure WebSocket (wss://):**
- Enable "WS enabled"
- Set "WS port" (default: 9002)
- Set "WS password" (required)
- Optionally configure TLS certificate/key

**Non-Secure WebSocket (ws://):**
- Enable "WS Nonsecure"
- Set "WS NS port" (default: 9003)

### 2. Enable HTTP Server

Still in `/web`:

**For HTTP (ws://):**
- Enable "HTTP enabled"
- Set "HTTP port" (default: 9000)

**For HTTPS (wss://):**
- Enable "HTTPS enabled"
- Set "HTTPS port" (default: 9001)
- Requires TLS cert/key configured

### 3. Access the Web Interface

Open in your browser:
- HTTP: `http://your-server:9000`
- HTTPS: `https://your-server:9001`

Enter your WebSocket password to authenticate.

## Features

### Full MUD Client

The web interface provides a complete MUD experience:

- **ANSI color rendering**: Xubuntu Dark palette, 256-color and true color
- **Shade character blending**: ░▒▓ rendered with proper color blending
- **Clickable URLs**: Links are cyan, underlined, open in new tab
- **More-mode pausing**: Tab to release, same as console
- **Command history**: Ctrl+P/N navigation
- **Multiple worlds**: Full world switching support

### Toolbar

The toolbar at the top provides quick access:

**Left side:**
- Hamburger menu (☰)
- PgUp button
- PgDn button

**Right side:**
- ▲ Previous world
- ▼ Next world

**Font slider**: Adjust text size

### Hamburger Menu

Click the hamburger icon for:

| Option | Description |
|--------|-------------|
| Worlds List | Show connected worlds |
| World Selector | Open world selector popup |
| Actions | Open actions editor |
| Settings | (Android) Open server settings |
| Toggle Tags | Show/hide MUD tags (F2) |
| Toggle Highlight | Show action pattern matches (F8) |
| Resync | Request full state refresh |
| Clay Server | (Android) Disconnect and reconfigure |

### World Selector

Access via hamburger menu or `/worlds` command:

- Filter worlds by name/hostname
- Arrow keys to navigate
- Enter to switch
- Shows connection status

### Connected Worlds List

Access via hamburger menu or `/connections`:

- Shows all connected worlds
- Unseen count per world
- Arrow keys to navigate
- Enter to switch

### Actions Editor

Access via hamburger menu or `/actions`:

- Full action editing capability
- Same interface as console
- Create, edit, delete actions

## Keyboard Shortcuts

| Keys | Action |
|------|--------|
| `Up/Down` | Switch between active worlds |
| `PageUp/PageDown` | Scroll output |
| `Tab` | Release screenful when paused; scroll down otherwise |
| `Escape+j` | Jump to end |
| `Ctrl+P/N` | Command history |
| `Ctrl+U` | Clear input |
| `Ctrl+W` | Delete word |
| `Ctrl+A` | Move to start of line |
| `Alt+Up/Down` | Resize input area |
| `F2` | Toggle MUD tags |
| `F4` | Open filter popup |
| `F8` | Toggle action highlighting |
| `Enter` | Send command |
| `Escape` | Close popup |

## Mobile Support

The web interface is optimized for mobile devices:

### Layout Adjustments

- Fixed toolbar stays visible during scrolling
- Uses `100dvh` for proper mobile viewport
- Smooth scrolling on iOS
- Proper keyboard handling

### Touch Controls

- Tap toolbar buttons for common actions
- Swipe to scroll output
- Long-press for text selection

### Visibility Handling

- Auto-resync when tab becomes visible
- Handles sleep/wake properly
- Reconnects if connection dropped

## Security

### Password Protection

- WebSocket password required for all connections
- Password hashed with SHA-256 before transmission
- Empty password disables the server

### Allow List / Whitelisting

Configure "WS Allow List" in `/web` as a CSV of IP addresses:

```
192.168.1.100,192.168.1.101
```

**Whitelisting behavior:**

1. Client from allow-list IP connects → must authenticate with password
2. After successful auth → that IP is whitelisted
3. Future connections from that IP → auto-authenticated
4. Different allow-list IP authenticates → previous whitelist cleared
5. Non-allow-list IPs must always use password

**Use case:** Authenticate once from home, then reconnect without password. Moving locations automatically requires re-authentication.

### TLS/SSL

For secure connections:

1. Obtain TLS certificate and key
2. Configure paths in `/web`:
   - TLS Cert File
   - TLS Key File
3. Enable "WS Use TLS"
4. Use HTTPS and wss:// URLs

## Cross-Interface Sync

The web interface stays synchronized with console and GUI:

| Event | Behavior |
|-------|----------|
| Output arrives | All clients receive it |
| World switched | All clients notified |
| Unseen cleared | Broadcast to all clients |
| Activity count | Broadcast when changed |

Each client can independently:
- View different worlds
- Have different scroll positions
- Use different tag visibility

## Troubleshooting

### Can't Connect

1. Verify server is running (check Clay console)
2. Check firewall allows the port
3. Verify password is correct
4. Try non-secure WebSocket first (ws://)

### Connection Drops

1. Check network stability
2. Enable keepalive in world settings
3. Use "Resync" from hamburger menu

### Display Issues

1. Try different browser
2. Clear browser cache
3. Use "Resync" to refresh state

\newpage

