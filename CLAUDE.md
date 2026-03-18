# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Debug Output

**IMPORTANT: Never write debug output to stdout or stderr (no `println!`, `eprintln!`, `dbg!`).**

Debug output interferes with the TUI and corrupts the terminal display. Instead:
- Use `debug_log(true, msg)` for always-on logging or `debug_log(is_debug_enabled(), msg)` for user-toggled debug (writes to `clay.debug.log`)
- Use `output_debug_log(msg)` for output/seq debugging (writes to `clay.output.debug`)
- Display messages in the output area using `add_tf_output()` or `add_output()`

## Build Commands

**IMPORTANT: Always use the musl debug build. Never use release builds.**

```bash
# Default build command (always use this)
cargo build --target x86_64-unknown-linux-musl --no-default-features --features rustls-backend

# Output: target/x86_64-unknown-linux-musl/debug/clay
```

Other commands:
```bash
cargo build --features webview-gui   # Build with webview GUI client
cargo test                           # Run all tests
cargo test test_name                 # Run a single test
cargo clippy                         # Lint
```

**Why musl:** glibc static builds cause SIGFPE crashes during DNS resolution. **Why rustls:** native-tls requires OpenSSL cross-compilation for musl.

### Cross-Platform Builds

- **Termux:** `cargo build --no-default-features --features rustls-backend` (no musl target). GUI requires X11 patches via `./patches/apply-patches.sh`. No hot reload, TLS proxy, or Ctrl+Z on Android.
- **macOS:** `cargo build --no-default-features --features rustls-backend` (no musl). Universal binary via `lipo` combining x86_64-apple-darwin and aarch64-apple-darwin targets.
- **Windows:** `cargo build --target x86_64-pc-windows-gnu --features webview-gui`

## Architecture

Clay is a terminal-based MUD client built with ratatui/crossterm for TUI and tokio for async networking. Supports multiple simultaneous world connections with SSL/TLS.

### Key Structs

- **App**: Central state - worlds, input area, spell checker, settings, popups, keybindings, TF engine
- **World**: Per-connection state - output buffer, scroll position, connection, unseen lines, pause state, prompt
- **InputArea**: Input buffer with viewport scrolling, command history, cursor position, kill ring
- **PopupManager**: Unified popup system for all modal dialogs
- **WebSocketServer**: Embedded WebSocket server for GUI/web/remote console clients

### Files

**Core:**
- `src/main.rs` - Main application (TUI, event loop, world management, connections, rendering)
- `src/encoding.rs` - Character encoding, colored emoji, console Theme enum, URL OSC8 wrapping
- `src/input.rs` - Input area with viewport scrolling, cursor positioning, display width helpers
- `src/actions.rs` - Action/trigger system (pattern matching, command execution, capture groups)
- `src/telnet.rs` - Telnet protocol negotiation and option handling
- `src/persistence.rs` - Settings save/load (`~/.clay.dat` INI format)
- `src/daemon.rs` - Daemon/headless mode, background connection logic
- `src/keybindings.rs` - Configurable keyboard bindings, load/save `~/.clay.key.dat`

**Networking:**
- `src/websocket.rs` - WebSocket server, message types, client management
- `src/http.rs` - HTTP/HTTPS web server (3 handler implementations: native-tls, rustls, plain)

**Theme:**
- `src/theme.rs` - ThemeColors (42 customizable color vars), ThemeFile for `~/clay.theme.dat`. GUI/web only; console uses Theme enum from encoding.rs.

**Popup System:**
- `src/popup/mod.rs` - Unified popup system (PopupManager, field types, layout)
- `src/popup/console_renderer.rs` - Console/TUI popup rendering
- `src/popup/definitions/` - Individual popup definitions

**TF (TinyFugue) Engine:**
- `src/tf/mod.rs` - TF engine state, variable storage, macro registry
- `src/tf/parser.rs` - Command parsing/routing, command implementations
- `src/tf/macros.rs` - Macro definition, trigger matching, execution
- `src/tf/builtins.rs` - Built-in TF commands (/echo, /set, /load, /quote, etc.)
- `src/tf/control_flow.rs` - /if, /while, /for block execution
- `src/tf/expressions.rs` - Expression evaluator (arithmetic, string, regex, functions)
- `src/tf/variables.rs` - Variable substitution (%{var}, %Pn, positional params)
- `src/tf/hooks.rs` - Hook event system (CONNECT, DISCONNECT, LOAD, etc.)
- `src/tf/bridge.rs` - Bridge between TF engine and App

**Web Interface:**
- `src/web/index.html` - HTML template
- `src/web/style.css` - CSS styles (uses `var(--theme-*)` CSS variables)
- `src/web/app.js` - JavaScript client
- `src/web/theme-editor.html` - Browser-based theme editor
- `src/web/keybind-editor.html` - Browser-based keybind editor

**Data Files:**
- `~/.clay.dat` - Settings file (INI format with `[global]` and `[world:name]` sections)
- `~/clay.theme.dat` - Theme file (INI format with `[theme:name]` sections)
- `~/.clay.key.dat` - Keyboard bindings (INI, only non-default bindings saved)

### Key Design Patterns

- **Borrow checker with App**: Clone data out of App before mutable borrows (e.g., ThemeColors, world info for WebSocket)
- **WebSocket InitialState**: `build_initial_state()` sends only `output_lines` (NOT pending_lines) to avoid duplicates. Partial lines excluded from broadcasts.
- **Hot reload**: `exec()` replaces process, TCP socket fds preserved via cleared FD_CLOEXEC. TLS connections need proxy process.
- **Console rendering**: `render_output_crossterm()` bypasses ratatui for output area (raw crossterm for ANSI fidelity). Popups use ratatui.
- **Web content**: HTML/CSS/JS embedded via `include_str!()` in http.rs. Changes require rebuild + `/reload`.
- **TF control flow in macros**: Body split by `%;`, control flow blocks grouped by `split_body_preserving_control_flow()`. Plain text in loop bodies becomes `SendToMud`, queued in `engine.pending_commands`.
- **Async lock access**: Use `try_read()`/`try_write()` on tokio RwLock from sync code, never `blocking_read()`/`blocking_write()` inside async runtime.

### /release Skill

Automated multi-platform build and GitHub release. Invoke with `/release [version]`.

Skill files: `.claude/skills/release/SKILL.md` (instructions), `.claude/skills/release/machines.md` (machine details).
