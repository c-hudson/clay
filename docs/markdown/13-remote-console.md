# Remote Console Client

Clay includes a remote console client that provides the full terminal interface while connecting to a master Clay instance via WebSocket.

## Running

```bash
./clay --console=hostname:port
```

Examples:
```bash
./clay --console=localhost:9000         # Local instance (default port: 9000)
./clay --console=mud.server.com:9000    # Remote server
```

No special build features required - works with the standard musl build.

## Use Cases

- Access your MUD sessions from another terminal/SSH
- Run Clay on a server, connect from anywhere
- Have multiple terminal views of the same sessions

## What "Master Instance" Means: -D vs --multiuser

A remote console/GUI/web client always connects to some other Clay process's
WebSocket server. That process is one of two things:

- **`-D` (headless daemon)**: an ordinary Clay instance — the same worlds, settings,
  and single-user config file (`~/.clay/settings.dat`) as running `./clay` normally —
  just without a local terminal UI. Every connecting client shares the one account and
  the one set of worlds.
- **`--multiuser`**: a genuinely multi-account server with its own config file
  (`~/.clay/multiuser.dat`, `[user:name]`/`[world:name:owner]`/`[action:name:owner]`
  sections). Each user authenticates with their own username and password and sees
  only worlds/actions they own; per-user telnet negotiation, character encoding,
  GMCP/MSDP/MSSP, and echo-based password masking are all tracked separately per
  connection, not shared globally. It is not a bare or output-only mode — it runs the
  same protocol handling as a single-user instance, just scoped per account.

```bash
./clay -D           # headless daemon, single account
./clay --multiuser  # headless multiuser server, many accounts
```

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
| `Up/Down` | Move the cursor within a multi-line input |
| `Esc-{` / `Esc-}` | Switch active worlds |
| `Esc-Left` / `Esc-Right` | Switch connected worlds |
| `Shift+Up/Down` | Switch all worlds |
| `PageUp/PageDown` | Scroll output |
| `Tab` | More-mode release / scroll |
| `Escape+j` | Jump to end |
| `Ctrl+P/N` | Command history |
| `Ctrl+U` | Delete from the cursor back to the start of the line |
| `Ctrl+Up/Down` | Previous/next command history |
| `F1` | Help popup |
| `F2` | Toggle MUD tags |
| `F4` | Filter popup |
| `F8` | Action highlighting |
| `Ctrl+L` | Redraw screen, keeping only server output |

These clients share the console's keymap, so the full table in
`07-keyboard-shortcuts.md` applies: chords (`^X^R`), the numeric repeat prefix
(`Esc-3 Esc-d` deletes three words), insert/overwrite mode (`Insert` or `Esc-v`),
`Esc-Tab` completion, and any key you bind server-side with `/bind` or a
`key_<name>` macro. Rebind anything in `~/.clay/keybindings.dat` or the browser
keybind editor.

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

No GUI features needed — the standard build in the **Installation** chapter works:

```bash
cargo build --target x86_64-unknown-linux-musl \
    --no-default-features --features rustls-backend,ssh-transport
```

## Example Setup

### Server Side (Master)

```bash
# Start Clay normally
./clay

# Enable the web/WebSocket server in /web settings:
# - Port: 9000
# - Password: your_password
```

### Client Side (Remote)

```bash
# Connect from anywhere
./clay --console=your-server.com:9000

# Enter password when prompted
```

## SSH Tunnel (--ssh)

Clay can tunnel `--console=`/`--gui=` over SSH itself instead of relying on a manual
port-forward — pass `--ssh` and give the target in
`[user@]host[:clayport[:sshport]]` form:

```bash
./clay --console=your-server.com --ssh
```

`clayport` defaults to 9000 (Clay's own WebSocket/HTTP port) and `sshport` to 22; the
user defaults to the local OS username. Clay tries an SSH agent first, then the
default `~/.ssh/id_*` key files — console mode may prompt for a passphrase, GUI mode
fails closed instead of prompting. The tunneled connection is plain `ws://` inside the
SSH channel (SSH already provides confidentiality/integrity end to end; no TLS cert is
needed or presented). See `SECURITY-NOTES.md` and `src/ssh.rs` for the full security
model, and the Android app's SSH tunnel feature for the mobile equivalent.

## Tips

### Manual SSH Forwarding

If you'd rather forward the port yourself instead of using `--ssh`:

```bash
# On client machine
ssh -L 9000:localhost:9000 your-server

# Then connect locally
./clay --console=localhost:9000
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

