# WebView GUI

Clay's graphical client is not a separate native toolkit application — it is a native
window (built with [wry](https://github.com/tauri-apps/wry)/[tao](https://github.com/tauri-apps/tao))
that hosts the exact same HTML/CSS/JS interface the browser-based **Web Interface**
chapter describes. Building the `webview-gui` feature gives you a `--gui` flag that opens
this window instead of (or alongside) the terminal UI. An earlier, different Rust GUI
toolkit and Cargo feature name are no longer part of the codebase — if you find a build
instruction or dependency list elsewhere referring to one, it's stale.

`--gui` runs in one of two modes:

- **Master mode** (`--gui`, no address): runs the whole `App` headlessly in this same
  process and opens a WebView window connected to it over a local, loopback-only
  WebSocket. This is the default startup mode on Windows and macOS (no flag needed) —
  Linux/Termux default to `--console`.
- **Remote mode** (`--gui=host[:port]`): opens a WebView window that connects to an
  *already-running* Clay instance elsewhere, exactly like the remote console in the next
  chapter, just rendered in a native window instead of a terminal. Default port: 9000.

## Building

Requires the `webview-gui` feature (`wry`, `tao`, and on Linux `webkit2gtk`/`gdk`/`gtk`/
`gio`/`glib`/`x11rb` — see `Cargo.toml`'s `[features]`). This is a display-dependent
feature and cannot be built or run in a headless environment (the standard musl build
in the **Installation** chapter deliberately omits it for that reason).

```bash
# Linux (requires GTK/WebKit dev libraries)
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libasound2-dev
cargo build --release --features webview-gui

# macOS (no extra dependencies needed)
cargo build --release --features webview-gui

# Windows (MSVC, uses WebView2; static CRT linking avoids a vcruntime140.dll dependency)
set RUSTFLAGS=-C target-feature=+crt-static
cargo build --release --features webview-gui
```

`native-audio` (ANSI music / MSP sound playback) is on by default, so the commands above
already include it — that's why the Linux line also installs `libasound2-dev`. See
**ANSI Music** for how audio actually gets from the MUD to your speakers.

Termux/Android needs [Termux:X11](https://github.com/termux/termux-x11) plus the X11
patches in `./patches/apply-patches.sh`; the pre-built `clay-termux-aarch64` release
binary already has these applied. Hot reload, the TLS proxy, and Ctrl+Z are unavailable
on Android regardless of GUI support.

## Running

```bash
./clay --gui                    # Master mode: local instance + WebView window
./clay --gui=hostname:port      # Remote mode: connect to a running Clay instance
./clay --gui=hostname --ssh     # Remote mode, tunneled over SSH (see next chapter)
```

## Login

Master mode authenticates automatically — there is no password prompt, since the
WebView is talking to the instance it was just launched alongside. Remote mode shows
the same password screen as the web interface (see **Web Interface**) unless the
connecting address is whitelisted, in which case it auto-connects.

## Interface

The WebView renders the identical page the web interface serves — toolbar, hamburger
menu, world selector, actions editor, status panel, filter popup, and the optional
world-tabs ribbon are all the same code and behave the same way. See the **Web
Interface** chapter for the full description; this section only covers what's
different about running it as a native window instead of a browser tab:

- No browser chrome — no URL bar, tabs, or bookmarks; just the app's own toolbar.
- No browser autoplay restrictions on ANSI music/audio.
- Right-click in the output area opens a **Debug Selection** context menu item (see
  below) that the plain browser client doesn't have.

## Keyboard Shortcuts

These clients share the console's keymap, so the full table in
`07-keyboard-shortcuts.md` applies: chords (`^X^R`), the numeric repeat prefix
(`Esc-3 Esc-d` deletes three words), insert/overwrite mode (`Insert` or `Esc-v`),
`Esc-Tab` completion, and any key you bind server-side with `/bind` or a
`key_<name>` macro. Rebind anything in `~/.clay/keybindings.dat` or the browser
keybind editor.

There are no GUI-only menu shortcuts beyond what's listed in the Web Interface
chapter's table. Use the commands themselves (`/worlds`, `/worlds -e`, `/setup`,
`/disconnect`) or the hamburger menu.

## Filter Popup

Press `F4` to open the filter popup:

- Type text to filter output (case-insensitive)
- Only matching lines are shown
- ANSI codes stripped for matching, preserved in display
- `Esc` or `F4` to close

## Debug Selection

For troubleshooting ANSI color issues, right-click a selection in the output area:

1. Highlight text in the output area
2. Right-click to open the WebView's own context menu
3. Select "Debug Selection"
4. A popup shows the raw text with escape codes visible
5. ESC character shown as `<esc>`
6. Copy button available

This replaces WebKit's/WebView2's default context menu in the output area only —
elsewhere (the input box, popups) the platform's normal context menu still applies.

## Themes

Configure GUI theme in `/setup` (`GUI Theme` setting):
- **Dark**: Dark background, light text (default)
- **Light**: Light background, dark text

## Synchronization

The GUI stays synchronized with console and web clients:

- All output is shared
- Unseen counts sync
- Activity indicators match
- World switching is local to this client (doesn't affect other clients)

## Troubleshooting

### Build Errors

**"Could not find WebKit/GTK libraries"**
```bash
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev
```

**"Could not find ALSA"** (for audio)
```bash
sudo apt install libasound2-dev
```

### Connection Issues

1. In remote mode, verify the target Clay instance is running with its web server
   enabled (`/web`)
2. Check the port matches your `/web` configuration (default 9000)
3. Try connecting with the web interface first to verify the password

### Display Issues

Linux/Termux only — Windows uses WebView2 and macOS uses WKWebView, neither of which
needs a display server:

1. Ensure X11 (or Termux:X11) is running
2. Check the `DISPLAY` environment variable
3. Try the other theme (dark vs light)

### No Sound

1. Verify the binary was built with the `native-audio` feature (on by default unless
   `--no-default-features` was used)
2. Check ANSI Music is enabled in `/setup`
3. Use `/testmusic` to verify audio works

\newpage
