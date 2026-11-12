# WebView GUI

Clay's graphical client is not a separate native toolkit application — it is a native
window (built with [wry](https://github.com/tauri-apps/wry)/[tao](https://github.com/tauri-apps/tao))
that hosts the exact same HTML/CSS/JS interface the browser-based **Web Interface**
chapter describes. Building the `webview-gui` feature gives you a `--web-gui` flag that opens
this window instead of (or alongside) the terminal UI. An earlier, different Rust GUI
toolkit and Cargo feature name are no longer part of the codebase — if you find a build
instruction or dependency list elsewhere referring to one, it's stale.

## Two GUIs

Clay has two graphical clients, and the flags tell them apart:

- **`--gui`** is the native GUI: its own window, drawn with GPUI rather than a web engine
  (the `native-gui` feature; it needs Rust 1.98 or newer to build and is in the release GUI
  binaries for Linux, Windows and macOS). Alone it runs Clay in this process; `--gui=host[:port]`
  attaches it to a Clay running elsewhere, and `--ssh` tunnels that connection. It is what
  Clay starts with no flag at all, wherever a window can be shown (see below). A build without it (the Termux GUI binary, or a
  `--features webview-gui` build of your own) opens the WebView GUI for `--gui` instead.
- **`--web-gui`** is the WebView GUI this chapter describes: the web interface in a window. It
  stays available as the fallback, and for anything the native GUI does not do yet.

### With no flag

`clay` with none of `--gui`, `--web-gui` or `--console` opens the GUI wherever it can, and the
console everywhere else:

- The build must have a GUI. The static Linux console binary (`clay-linux-x86_64-musl`) and
  the Termux no-GUI binaries always start the console.
- A terminal reached over SSH gets the console (sshd's `SSH_CONNECTION`, `SSH_CLIENT` or
  `SSH_TTY` is set), on every system: a window would open on the far machine's screen, or crawl
  through X forwarding. `--gui` still opens one there if you want it.
- On Linux (and Termux, where that means termux-x11) a display server must be listening: the one `WAYLAND_DISPLAY` names, or
  else the one `DISPLAY` names. Clay only connects to it and lets go, which takes a
  millisecond or two. A `DISPLAY` left behind in a tmux session after its X server ended, or
  none at all (a text console, a server), gets the console.

Naming an interface always wins: `--gui` and `--web-gui` open the window and `--console` the
console whatever the check would say. A `/reload` or a crash restart keeps whichever
interface Clay is in, and `/reload --console` / `/reload --gui` switch it (see the Hot
Reload chapter).

`--web-gui` runs in one of two modes:

- **Master mode** (`--web-gui`, no address): runs the whole `App` headlessly in this same
  process and opens a WebView window connected to it over a local, loopback-only
  WebSocket. This is what Clay starts with no flag when the build has no native GUI (see
  above) and a window can be shown.
- **Remote mode** (`--web-gui=host[:port]`): opens a WebView window that connects to an
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
./clay --web-gui                    # Master mode: local instance + WebView window
./clay --web-gui=hostname:port      # Remote mode: connect to a running Clay instance
./clay --web-gui=hostname --ssh     # Remote mode, tunneled over SSH (see next chapter)
```

## Login

Master mode authenticates automatically — there is no password prompt, since the
WebView is talking to the instance it was just launched alongside. Other devices connect
to it with the password set in Settings › Web (see **Web Interface**); until one is set,
the GUI protects its server with a random password made fresh at each start, so only its
own window can log in. Remote mode shows the same password screen as the web interface
unless the connecting address is whitelisted, in which case it auto-connects.

## Interface

The WebView renders the identical page the web interface serves — toolbar, hamburger
menu, world selector, actions editor, status panel, filter popup, and the optional
world-tabs ribbon are all the same code and behave the same way. See the **Web
Interface** chapter for the full description; this section only covers what's
different about running it as a native window instead of a browser tab:

- No browser chrome — no URL bar, tabs, or bookmarks; just the app's own toolbar.
- No browser autoplay restrictions on ANSI music/audio.
- Right-click in the output area opens Clay's own menu, which the plain browser client
  doesn't have. The native GUI's output menu is the same.
  - **Open Link**, on a link only: opens it in your browser, as clicking it does.
  - **Copy**: on a link, its address; otherwise, or when you right-click on selected
    text, the selection.
  - **Paste**: the clipboard into the input line, exactly as the input's own Paste does.
    On macOS the WebView may ask you to confirm it.
  - **Debug Selection** (see below).
- Menu > Import Picture opens a file dialog, and a picture dragged onto the window is
  uploaded too (see **Importing a Picture** in the Interface chapter). The native GUI does
  the same; on Linux its file dialog needs xdg-desktop-portal, and without it a drag
  still works.

## On the Desktop

Both desktop GUIs reach a little past their own windows. None of it adds anything to the
window itself.

- **The hamburger's items, outside the window** (the native GUI only; the WebView GUI
  doesn't do this). Each choice acts on the window it was made for.
  - **Linux:** an app-menu applet on the panel (xfce4's AppMenu plugin, MATE's and
    Budgie's) shows a Clay menu with the hamburger's items, then Quit. KDE Plasma's Global
    Menu reads the same window properties. It needs an X11 session: Wayland has no way to
    say where a window's menu is. Without such an applet nothing changes.
  - **macOS:** the menu bar's Clay menu, and the Dock icon's right-click menu (without
    Quit, which the Dock has).
  - **Windows:** the top of the window's own menu: right-click the title bar, or
    Shift+right-click the taskbar button. Alt+Space doesn't open it, because Clay uses
    that key (`collapse_spaces`). There is no Quit, since that menu has Close.
- **Taskbar Alert** (Settings > General, off by default): when the MUD sends text while
  no Clay window has focus, the taskbar button flashes (X11, Windows) or the Dock icon
  bounces (macOS). It happens once until a Clay window has the focus again. GPUI can't do
  it on Wayland.
- **The activity count** (the status bar's ACT number) shows on the taskbar button or the
  Dock icon:
  - Windows: a red badge on the button;
  - macOS: the Dock's own badge;
  - Linux: KDE Plasma's task manager, Ubuntu Dock / Dash to Dock and Plank, through
    Unity's launcher API. xfce's panel tasklist shows none.
- **`/notify`** shows as a desktop notification titled with the world's name:
  - Linux: the desktop's notification service.
  - Windows: a toast from Clay. The first one registers the name `Clay` for toasts, one
    key under `HKCU\Software\Classes\AppUserModelId\Clay`.
  - macOS: through `osascript`, because `clay-macos-universal` isn't an .app bundle. The
    notification shows as Script Editor's, and clicking it opens Script Editor.
- **Linux launcher entry:** docks match a window to an application by its `.desktop`
  file, so at start either GUI writes `~/.local/share/applications/clay.desktop` (and
  Clay's icon into `~/.local/share/icons/hicolor/`), naming the program it was started
  from. This also puts Clay in the desktop's application menu. The file is written only
  when it is missing, or when it is Clay's own and names another program. A
  `clay.desktop` without the line `X-Clay-Generated=true` is yours and is never
  touched, and a build run from a cargo `target` directory never writes one.

## Keyboard Shortcuts

These clients share the console's keymap, so the full table in
`07-keyboard-shortcuts.md` applies: chords (`^X^R`), the numeric repeat prefix
(`Esc-3 Esc-d` deletes three words), insert/overwrite mode (`Insert` or `Esc-v`),
`Esc-Tab` completion, and any key you bind server-side with `/bind` or a
`key_<name>` macro. Rebind anything in `~/.clay/keybindings.dat` or the browser
keybind editor.

The native GUI (`--gui`) has one set of keys of its own: `Ctrl+Shift+=` (Ctrl+plus) and
`Ctrl+minus` zoom the window the way xfce4-terminal does - the text and the window grow or
shrink together, 20% a step and seven steps each way - and `Ctrl+0` returns to 1:1. A
window that would outgrow its monitor becomes the size of the screen instead - the whole
usable area, title bar and borders included, whichever edge it reached first - and
further steps do nothing until you zoom out again; zooming back to 1:1 restores the
window's original size and place. The zoom is this window's alone and lives in memory: it is never
saved, a `/window` zooms by itself, and another Clay is not affected. The chrome (status
bar, menus, tabs) keeps its size; the font slider still shows the saved text size, and the
text is drawn at that size times the zoom. If you drag the window to a new size while
zoomed, later steps scale that size. On Wayland a program cannot move its window, so a
window grown to the screen's size may need a nudge back on screen. The WebView GUI and a
browser tab have no window zoom (a browser's Ctrl+plus/minus stay its own page zoom).

There are no other GUI-only shortcuts beyond what's listed in the Web Interface chapter's
table. Use the commands themselves (`/worlds`, `/worlds -e`, `/setup`, `/disconnect`) or
the hamburger menu.

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
- **Dark**: the original dark interface (default)
- **Light**: the original light interface
- **Dark - New**: slate blues with a warm orange accent and higher-contrast text
- **Light - New**: a warm off-white interface with charcoal text and a terracotta accent
- any custom `[theme:name]` from `~/.clay/theme.dat`, made in the theme editor

The WebView draws the same chrome as the native GUI and the browser page (cards,
sidebar tiles, accent buttons; see the Interface chapter), and treats a theme as light
when its background colour is light, whatever its name.

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
