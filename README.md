<div align="center">
<img src="clay2.png" width="200" alt="Clay">

# Clay MUD Client
</div>

A terminal-based MUD (Multi-User Dungeon) client built with Rust featuring multi-world support, ANSI color rendering, spell checking, tinyfugue compatibility and remote viewing with a web interface and android app.

![Clay screenshot showing one instance with remote terminal, Firefox web client, and native WebView GUI](screenshot.png)
*A single Clay instance viewed simultaneously from a remote terminal, a Firefox web client, and a native WebView GUI.*

## Features

**Core MUD client.** Connect to multiple MUD servers at once, over SSL/TLS, with full ANSI
color and formatting (256-color, true color) and a complete telnet negotiation suite (SGA,
TTYPE, EOR, NAWS, MCCP2 compression, GMCP, MSDP). Configurable auto-login, unlimited
scrollback with more-mode pagination, command history, built-in spell checking, tab
completion for commands and action names, output search/filtering, an Emacs-style kill ring,
and per-world file logging round out the day-to-day experience.

**One server, viewed from anywhere.** The same running Clay instance can be viewed
simultaneously from the terminal TUI, a native WebView GUI (desktop), a browser over
WebSocket, a remote console attached from another terminal, and the Android app — which can
either connect to a remote Clay server or run a complete standalone server on the phone
itself with no configuration. Hot reload (`/reload`) updates the binary without dropping
connections, and crash recovery restores state automatically.

**Security & remote access.** Exposing Clay beyond localhost is gated by default: the web UI
only answers under a stealth path (`/clay/`), an optional IP allow-list hard-drops unlisted
connections before any handshake, Android proves a shared auth key to reach the server from
off-list addresses, outbound TLS is pinned on first use (trust-on-first-use) with a
confirmation prompt if a certificate ever changes, and repeated bad requests earn a ban. See
[SECURITY-NOTES.md](SECURITY-NOTES.md) for the full picture.

**Settings sync.** `/import` pulls worlds, actions, theme, and keybindings from another
running Clay instance, so setting up a new device doesn't mean re-entering everything by
hand.

**TinyFugue compatibility & scripting.** A full TF command layer (`/def`, `/set`, `/if`,
`/while`, `/for`, `/load`, etc., with `#` as an alternate prefix), pattern-matching
actions/triggers with regex or wildcards, auto-commands and startup actions, and direct
import of an existing `.tfrc` — if you know TF, you already know Clay.

**Customization.** Color themes and keybindings are fully configurable (INI files with
browser-based editors for live preview), fonts and hanging-indent wrap spacing are
adjustable, and mouse support is on by default in the console.

**Extras.** GMCP/MSDP structured data exchange (including server-driven sound/music via
Client.Media), BBS-style ANSI music playback, dictionary/Urban Dictionary/translation/URL-
shortening lookups, text-to-speech via local engines or Microsoft Edge neural TTS, a
per-world notes editor, a long-term SQLite scrollback archive with full-text search
(`/recall -D`), self-update from GitHub releases, grep-mode output search, and headless
daemon/multiuser server modes for shared or unattended deployments.

## Installation

### Pre-built Binaries

Download pre-built binaries from the [Releases](https://github.com/c-hudson/clay/releases) page:

| Platform | Binary | Notes |
|----------|--------|-------|
| Linux x86_64 (static) | `clay-linux-x86_64-musl` | TUI only, works on any Linux |
| Linux x86_64 (GUI) | `clay-linux-x86_64-gui` | TUI + WebView GUI + audio |
| Android | `clay-android.apk` | Remote client, or a full standalone server on-device |
| Termux ARM64 (GUI) | `clay-termux-aarch64` | TUI + WebView GUI, needs [Termux:X11](https://github.com/termux/termux-x11) |
| Termux ARM64 (TUI only) | `clay-termux-aarch64-nogui` | No GUI dependencies |
| Termux ARMv7 (TUI only, 32-bit) | `clay-termux-armv7-32bit-nogui` | For older 32-bit devices |
| macOS (Universal) | `clay-macos-universal` | GUI + audio, Intel & Apple Silicon |
| Windows x86_64 | `clay-windows-x86_64.exe` | GUI + audio |

```bash
# Example: Download and install on Linux
curl -L https://github.com/c-hudson/clay/releases/latest/download/clay-linux-x86_64-musl -o clay
chmod +x clay
./clay
```

## Building from Source

### Linux

```bash
# Static binary for any Linux (TUI only, no GUI)
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl --no-default-features --features rustls-backend,ssh-transport

# Build with WebView GUI + audio (requires GTK/WebKit dev libraries)
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libasound2-dev
cargo build --release --features webview-gui
```

### macOS

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Build with WebView GUI + audio
cargo build --release --features webview-gui
```

### Windows

```bash
# Install Rust from https://rustup.rs
# Install Visual Studio Build Tools (MSVC)

# Build with WebView GUI + audio (uses WebView2)
# Static CRT linking eliminates vcruntime140.dll dependency
set RUSTFLAGS=-C target-feature=+crt-static
cargo build --release --features webview-gui
```

### Termux (Android)

```bash
# Install Rust in Termux
pkg install rust

# Build TUI only (no GUI on Android)
cargo build --release --no-default-features --features rustls-backend,ssh-transport
```

## Usage

```bash
# Run the TUI client
./clay

# Run as WebView GUI client (connects to running Clay instance)
./clay --gui=hostname:port

# Run as remote console client
./clay --console=hostname:port

# Run as headless daemon server
./clay -D

# Run as multiuser server
./clay --multiuser

# Search world output history (glob pattern)
CLAY_PASSWORD=pass ./clay --grep=hostname:port '*tells you*'

# Follow live output matching a pattern (like tail -f | grep)
CLAY_PASSWORD=pass ./clay --grep=hostname:port -f '*combat*'

# Use custom config file (default: ~/.clay/settings.dat)
./clay --conf=/path/to/config.dat
```

## Commands

**General:**

| Command | Description |
|---------|-------------|
| `/help [topic]` | Show help (or topic-specific help) |
| `/version` | Show version info |
| `/quit` | Exit the client |
| `/reload` | Hot reload the binary |
| `/update [-f]` | Download and install latest release |
| `/menu` | Open menu popup |

**Worlds & Connections:**

`/world` and `/worlds` are interchangeable aliases.

| Command | Description |
|---------|-------------|
| `/worlds` | Open world selector popup |
| `/worlds <name>` | Connect to or switch to a world |
| `/worlds -e [name]` | Edit world settings — creates the world first if `name` doesn't exist yet |
| `/worlds -l <name>` | Connect to world without running auto-login |
| `/worlds -b <name>` | Connect to world in background without switching to it |
| `/addworld <name> [host port]` | Add/update a world (TF-compatible) |
| `/connections` or `/l` | List connected worlds |
| `/connect [host port [ssl]]` | Connect to a server |
| `/disconnect` or `/dc` | Disconnect current world |
| `/send [-w world] text` | Send text to a world |
| `/flush` | Clear output buffer for current world |
| `/window [world]` | Open new GUI/browser window |
| `/window --grep <pat> [-w world]` | Open grep results window (searchs scrollback + live) |

**Settings & UI:**

| Command | Description |
|---------|-------------|
| `/setup` | Open global settings |
| `/web` | Open web/WebSocket settings |
| `/import [host[:port]]` | Pull worlds, actions, theme, and keybindings from another Clay instance |
| `/actions [world]` | Open actions/triggers editor |
| `/edit [file]` | Open split-screen notes editor |
| `/edit -l` | Open notes list popup |
| `/font` | Font settings popup (web/GUI only) |
| `/tag` | Toggle MUD tag display with timestamps (same as F2) |
| `/say <text>` | Speak text via TTS (uses configured TTS mode) |

**Search & Archive:**

| Command | Description |
|---------|-------------|
| `/recall [options] [range] [pattern]` | Search output/input history (see `/help recall` for the full option list) |
| `/recall -D <pattern>` | Search the long-term scrollback archive (requires "Archive Output" in `/setup`) |

**Lookup & Utility:**

| Command | Description |
|---------|-------------|
| `/dict <word>` | Look up word definition (Free Dictionary API) |
| `/urban <word>` | Look up word definition (Urban Dictionary) |
| `/translate <lang> <text>` | Translate text (also `/tr`) |
| `/url <url>` | Shorten a URL (is.gd) |

Lookup commands place the result in the input buffer with the cursor at the start, so you can type a prefix (e.g. `say`) before sending.

**Remote & Admin:**

| Command | Description |
|---------|-------------|
| `/remote` | List remotely connected clients |
| `/remote --kill <id>` | Disconnect a remote client |
| `/ban` | Show banned hosts |
| `/unban <host>` | Remove a ban |
| `/notify <msg>` | Send notification to Android app |

**Debug:**

| Command | Description |
|---------|-------------|
| `/testmusic` | Play test ANSI music sequence |
| `/dump` | Dump scrollback buffers to `~/.clay/dump.log` |

## TinyFugue Commands

Clay includes a TinyFugue compatibility layer. All TF commands work with both `/` and `#` prefixes:

| Command | Description |
|---------|-------------|
| `/set name value` | Set a variable |
| `/echo message` | Display local message |
| `/def name = body` | Define a macro/trigger |
| `/if (expr) cmd` | Conditional execution |
| `/while (expr) ... /done` | While loop |
| `/for var start end ... /done` | For loop |
| `/bind key = cmd` | Bind key to command |
| `/load filename` | Load a TF script file |
| `/tfhelp [topic]` | Show TF help |

The `#` prefix also works for backward compatibility. See `/tfhelp` for full command list.

### Importing TinyFugue Worlds

If you have an existing TinyFugue configuration, you can import your worlds using `/load`:

```bash
/load ~/.tfrc
```

Clay will parse `/addworld` commands from your TF config file and create corresponding worlds. This makes migrating from TinyFugue seamless - your existing world definitions are automatically imported.

## Controls

All keybindings are configurable via `~/.clay/keybindings.dat`. Defaults follow TinyFugue conventions. A browser-based keybind editor is available at `/keybind-editor`.

**World Switching:**

| Key | Action |
|-----|--------|
| `Ctrl+Up/Down` | Switch between active worlds |
| `Shift+Up/Down` | Cycle through all worlds |
| `Escape w` | Switch to world with activity |

**Input Editing:**

| Key | Action |
|-----|--------|
| `Left/Right` | Move cursor one character |
| `Ctrl+B/F` | Move cursor left/right one character |
| `Escape b/f` | Move cursor one word left/right |
| `Up/Down` | Move cursor up/down (multi-line input) |
| `Ctrl+A` / `Home` | Jump to start of line |
| `Ctrl+E` / `End` | Jump to end of line |
| `Ctrl+U` | Clear line |
| `Ctrl+W` | Delete word backward |
| `Ctrl+K` | Kill to end of line |
| `Ctrl+D` | Delete character forward |
| `Ctrl+Y` | Yank (paste from kill ring) |
| `Ctrl+T` | Transpose two characters before cursor |
| `Ctrl+V` | Insert next character literally (console only) |
| `Ctrl+P/N` | Previous/next command history |
| `Ctrl+Q` | Spell suggestions |
| `Ctrl+G` | Terminal bell |
| `Tab` | Command completion (when input starts with `/`) |
| `Escape Space` | Collapse multiple spaces to one |
| `Escape -` | Jump to matching bracket `()[]{}` |
| `Escape .` / `_` | Insert last word from previous history |
| `Escape p` | Search history backward by prefix |
| `Escape n` | Search history forward by prefix |
| `Escape Backspace` | Delete word backward (punctuation-delimited) |
| `Escape c/l/u` | Capitalize / lowercase / uppercase word |
| `Escape d` | Delete word forward |
| `Alt+Up/Down` | Resize input area (1-15 lines) |

**Kill Ring:** `Ctrl+K`, `Ctrl+U`, `Ctrl+W`, `Escape d`, and `Escape Backspace` push deleted text to the kill ring. `Ctrl+Y` pastes the most recent entry.

**Output & Scrollback:**

| Key | Action |
|-----|--------|
| `PageUp/PageDown` | Scroll output history |
| `Tab` | Release one screenful (when paused) |
| `Escape j` | Jump to end, release all pending |
| `Escape J` | Selective flush: keep highlighted pending, discard rest |
| `Escape h` | Half-page scroll up or release half screenful |
| `Ctrl+L` | Redraw screen (keep only server data) |

**General:**

| Key | Action |
|-----|--------|
| `Ctrl+R` | Hot reload |
| `F1` | Help |
| `F2` | Toggle MUD tag display with timestamps |
| `F4` | Filter/search output |
| `F8` | Toggle action highlighting |
| `F5` | Search command history (web/GUI) |
| `F9` | Toggle GMCP media audio |
| `Ctrl+C` (x2) | Quit |

**Mouse (enabled by default via "Console Mouse" in `/setup`):**
- Click popup buttons and fields to interact
- Click list items to select them
- Scroll wheel to scroll lists and scrollable content in popups
- Click and drag to highlight lines in scrollable content

## Android App

On first launch, the Android app (`clay-android.apk`) asks how you want to run:

- **Run on This Phone** — spawns a complete Clay server on-device (`--local-server`),
  loopback-only with a random password generated per launch. No configuration needed; this
  is the easiest way to try Clay on a phone. Hot reload and the TLS proxy aren't available
  in this mode.
- **Connect to a remote server** — the app becomes a WebSocket client of a Clay instance
  running elsewhere:
  1. Run Clay on a server/computer with WebSocket enabled (`/web` settings)
  2. Install the APK and enter the server address and WebSocket password
  3. Connect to control your MUD sessions remotely

Either way you get the full MUD client interface, push notifications via `/notify`, and a
background service that keeps the connection alive. The mode can be changed later in the
app's settings, and it works alongside the native Termux binary if you'd rather run Clay
directly in Termux.

## Web Interface

Enable in `/web` settings:

1. Set `HTTP enabled` to Yes (default port: 9000)
2. Set a `WS password` (required for authentication)
3. Optionally enable `Secure` for HTTPS (auto-generates self-signed certs)

From `localhost`, access via browser at `http://localhost:9000`. From any other machine,
the UI is served only under the stealth path `http://yourhost:9000/clay/` by default — see
[Security](#security) below.

## Security

Clay is gated against unknown callers by default once you expose it beyond localhost:

- **Stealth web path** — the web UI answers only under `/clay/` (configurable, empty
  restores the old behavior); every other path is silently dropped for non-localhost
  connections, with no response at all.
- **IP allow-listing** — set a **WS Allow List** in `/web` and non-listed addresses are
  dropped at the TCP level, before any TLS handshake or HTTP response.
- **CLAY-KNOCK** — the Android app can prove a shared auth key to reach the server from an
  address that isn't on the allow list, without opening it up to everyone.
- **TLS certificate pinning (TOFU)** — outbound connections (to MUDs, remote consoles, the
  WebView proxy) pin the server's certificate on first use in `~/.clay/known_hosts.dat`
  rather than relying on a CA; if the certificate ever changes, the connection blocks and
  asks you to confirm the new one.
- **Ban list** — repeated bad requests or failed logins earn a ban, with allow-listed
  addresses exempted from bans caused by stale bookmarks or protocol typos.

See [SECURITY-NOTES.md](SECURITY-NOTES.md) for exactly what changes, what (if anything)
might break, and how to opt back into the old, fully-open behavior if you need to.

## Actions/Triggers

Actions match incoming MUD output against patterns and execute commands:

1. Open `/actions` to create triggers
2. Set a pattern (regex or wildcard) — leave empty for manual-only actions
3. Set command(s) to execute when matched (semicolon-separated)
4. Use `$1`-`$9` for captured groups, `$0` for full match
5. Use `/gag` in commands to hide matched lines, `/highlight` to color them

Example: Pattern `* tells you: *` with command `/echo Got tell from $1`

Actions can also be invoked manually by typing `/actionname` in the input. Enable "Startup" on an action to run its commands on Clay start, reload, and crash recovery.

## Themes

Clay supports customizable color themes for the GUI and web interfaces:

- Theme file: `~/.clay/theme.dat` (INI format with `[theme:name]` sections)
- Browser-based theme editor included for live color preview
- Select themes in `/setup` (GUI Theme setting)
- Console uses separate dark/light theme toggle

## Text-to-Speech

Clay can speak incoming MUD output and text aloud. Configure TTS mode in `/setup`:

- **Off** — TTS disabled (default)
- **Local** — Uses system TTS: `espeak` on Linux, `say` on macOS, PowerShell on Windows
- **Edge** — Microsoft neural TTS via cloud API; higher quality, requires internet

Use `/say <text>` to speak text immediately regardless of TTS mode. A per-world speaker whitelist controls which character names trigger automatic TTS. Web and Android clients use the browser's built-in Web Speech API.

## Keybindings

All keyboard shortcuts are configurable via `~/.clay/keybindings.dat` (INI format). Only non-default bindings need to be saved — defaults follow TinyFugue conventions.

```ini
[bindings]
Up = world_next
Down = world_prev
Ctrl-Up = UNBOUND
```

Use `UNBOUND` to remove a default binding. A browser-based keybind editor is available at `/keybind-editor` when the HTTP server is enabled.

## Configuration

Settings are stored in `~/.clay/settings.dat` (`~/clay/settings.dat` on Windows). If you're
upgrading from an older Clay that used `~/.clay.dat`/`~/.clay.key.dat`/`~/clay.theme.dat`,
those legacy dotfiles are migrated into `~/.clay/` automatically on first run. Per-world
settings include:

- Hostname, port, SSL toggle
- Username/password for auto-login
- Character encoding (UTF-8, Latin1, FANSI)
- Auto-login type (Connect, Prompt, MOO_prompt)
- Keepalive type (NOP, Custom, Generic)
- Log file path
- TTS mode (Off, Local, Edge) and speaker whitelist

## Importing Settings from Another Clay Instance

`/import [host[:port]]` opens a dialog (with the host pre-filled if you typed one) for
pulling worlds, actions, theme, and keybindings from another running Clay instance —
remote values win on conflicts, local-only entries are kept. This is the easiest way to
set up a new device: enter the address and password in the dialog and everything else
carries over.

## License

MIT
