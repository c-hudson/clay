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
- Set "WS port" (default: 9000)
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

### Status Panel

A collapsible panel below the output area shows the current world's GMCP `Char.*`/MSDP
data — the same data as the console's status line and `/stats` (see the **Interface
Overview** and **Commands** chapters), shared by the web interface, the native WebView GUI,
and the Android app since all three run the same interface code.

- **Appears only when there's data**, and disappears again on disconnect — there is no
  setting to show or hide it, only to collapse it. A world that never sends GMCP `Char.*` or
  MSDP data (most MOO and MUSH servers, for instance) simply never shows a panel.
- **Click the header to collapse or expand it.** The collapsed state is remembered per
  browser (`localStorage`), not synced between clients — collapsing it on your phone doesn't
  collapse it in a browser tab open to the same instance.
- A value paired with a maximum by name (e.g. `hp`/`maxhp`) renders as a gauge with a fill
  bar; everything else renders as a plain labelled row, unrecognized fields included. Field
  labels are cosmetically cleaned up for display (`race.name` shows as "Race Name"; common
  abbreviations like `hp`/`mp` stay upper-case) — the console's status line shows the raw
  field name instead.
- Switches with the current world along with the rest of the interface.

## Keyboard Shortcuts

| Keys | Action |
|------|--------|
| `Up/Down` | Move the cursor within a multi-line input |
| `Esc-{` / `Esc-}` | Switch between active worlds |
| `Esc-Left` / `Esc-Right` | Switch between connected worlds |
| `PageUp/PageDown` | Scroll output |
| `Tab` | Release screenful when paused; scroll down otherwise |
| `Escape+j` | Jump to end |
| `Ctrl+P/N` | Command history |
| `Ctrl+U` | Delete from the cursor back to the start of the line |
| `Ctrl+W` | Delete word |
| `Ctrl+A` | Move to start of line |
| `Ctrl+Up/Down` | Previous/next command history |
| `Alt+Up/Down` | Resize input area |
| `F2` | Toggle MUD tags |
| `F4` | Open filter popup |
| `F8` | Toggle action highlighting |
| `Enter` | Send command |
| `Escape` | Close popup |

These clients share the console's keymap, so the full table in
`07-keyboard-shortcuts.md` applies: chords (`^X^R`), the numeric repeat prefix
(`Esc-3 Esc-d` deletes three words), insert/overwrite mode (`Insert` or `Esc-v`),
`Esc-Tab` completion, and any key you bind server-side with `/bind` or a
`key_<name>` macro. Rebind anything in `~/.clay/keybindings.dat` or the browser
keybind editor.

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

## Reaching Clay from outside your network

Everything in this section is summarized live by `/reach` (console) and by the
**Remote Access** button in `/web` (all interfaces): your addresses, the Windows
Firewall verdict, the router mapping state, and exactly what to type on the other
device.

### 1. Same network (LAN)

Other devices on your Wi-Fi/LAN use the LAN address `/reach` shows:

| Client | What to enter |
|--------|---------------|
| Browser | `https://192.168.1.20:9000/clay/` (accept the one-time certificate warning) |
| Clay GUI | `clay --gui=192.168.1.20` |
| Clay console | `clay --console=192.168.1.20` |
| Android app | Host `192.168.1.20`, Port `9000` |

The `:9000` is implied when the port is the default.

### 2. Windows Firewall

Windows blocks incoming connections by default. The first time Clay's web server
starts, Windows shows a "Windows Defender Firewall has blocked some features"
alert. **Allow access** creates an Allow rule. **Cancel** creates a *Block* rule
for `clay.exe`, and a Block rule beats any Allow rule added later — this is the
most common reason a phone cannot reach a Windows PC.

Clay checks this for you: `/reach` reports the verdict (`allowed`, `no rule`,
`BLOCKED`), and Clay prints a one-line hint at startup when the verdict is bad.
**Add Firewall Rule** in Remote Access fixes it: one UAC prompt, after which Clay
removes every inbound rule for its own executable (the stale Block rule included)
and adds a per-program Allow rule named "Clay MUD Client". Notes:

- The UAC prompt appears on the Windows machine's own screen, even when the
  button was pressed from a phone or another Clay.
- The rule is per-program, not per-port, so changing the port never breaks it.
- It applies to all network profiles (Domain, Private, Public). Home networks
  are frequently classified "Public" by Windows, and Clay's stealth path,
  password, allow list and ban list are the real gate (see Security above).
- Manual equivalent, in an elevated prompt:
  `netsh advfirewall firewall add rule name="Clay MUD Client" dir=in action=allow program="C:\path\to\clay.exe" protocol=TCP enable=yes profile=any`
- On Linux and macOS Clay does not manage the firewall; open the port with
  `ufw`/`firewalld` or the macOS application firewall if one is active.

### 3. Your router

From the internet, connections arrive at your router's public address, and the
router must forward the port to the machine running Clay. Two ways:

**Port Mapping (UPnP)** — the toggle in Remote Access. Clay asks the router to
forward the port to this machine (UPnP IGD), renews the lease every 20 minutes,
and removes the mapping when you turn the toggle off, change the port, or quit.
A hot reload keeps it; a crash leaves it until the one-hour lease expires. The
router must have UPnP enabled. Off by default: it exposes the port to the whole
internet (see `SECURITY-NOTES.md` for what protects it).

**Manual port forward** — in the router's admin page, forward external TCP port
9000 to the LAN address `/reach` shows, port 9000. Give the Clay machine a fixed
LAN address (a DHCP reservation) so the forward keeps pointing at it.

Either way, the address to use from outside is your **public IP**. `/reach
--lookup` (or the Look Up Public IP button) asks `checkip.amazonaws.com` for it —
never automatically. Then: `clay --gui=203.0.113.5`, or the Android app's
Remote Host field.

**When it cannot work: double NAT / carrier-grade NAT.** If `/reach` shows the
router's WAN address as private (`10.x`, `172.16-31.x`, `192.168.x`, or
`100.64-127.x`), the router is itself behind another NAT — a second router, or
your ISP's carrier-grade NAT. No port forward on your router can reach the
internet then. Use option 4 or 5 instead.

### 4. SSH tunnel

If you can log in to any SSH host that can reach the Clay machine (the Clay
machine itself, or a box on its LAN), other Clays need no open port at all:

```bash
clay --gui=user@host --ssh
```

See the **Remote Console** chapter, "SSH Tunnel", for the details and the
Android app's SSH mode.

### 5. Mesh VPN (Tailscale, WireGuard, ZeroTier)

A mesh VPN gives every device a private address that works from anywhere, with
no port forwarding and no public exposure. Clay lists a Tailscale-style
`100.64.x.x` address on this machine as **VPN** in `/reach`; any device on the
same VPN connects to it like a LAN address. This is the simplest option when you
are behind carrier-grade NAT.

A reverse tunnel that lets the Clay machine publish itself through an SSH host
you own (no port opened anywhere) is planned.

## Cross-Interface Sync

The web interface stays synchronized with console and GUI:

| Event | Behavior |
|-------|----------|
| Output arrives | All clients receive it |
| World switched | All clients notified |
| Unseen cleared | Broadcast to all clients |
| Activity count | Broadcast when changed |
| Status data updates | Broadcast to all clients, coalesced to roughly one update per 150ms so a busy combat round doesn't flood the connection |

Each client can independently:
- View different worlds
- Have different scroll positions
- Use different tag visibility

## Troubleshooting

### Can't Connect

1. Verify server is running (check Clay console)
2. Run `/reach` — it shows the address to use, the firewall verdict and the
   router mapping state (see "Reaching Clay from outside your network")
3. Verify password is correct
4. A remote browser gets HTTPS with a self-signed certificate — accept the
   one-time warning

### Connection Drops

1. Check network stability
2. Enable keepalive in world settings
3. Use "Resync" from hamburger menu

### Display Issues

1. Try different browser
2. Clear browser cache
3. Use "Resync" to refresh state

\newpage

