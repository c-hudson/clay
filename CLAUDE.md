# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Critical Rules

**NEVER write debug output to stdout or stderr (no `println!`, `eprintln!`, `dbg!`).** Debug output corrupts the TUI. Use instead:
- `debug_log(true, msg)` for always-on logging (writes to `clay.debug.log`)
- `debug_log(is_debug_enabled(), msg)` for user-toggled debug
- `output_debug_log(msg)` for output/seq debugging (writes to `clay.output.debug`)
- `add_tf_output()` to display messages in the output area

**Always use the musl debug build. Never use release builds.**

## Build Commands

```bash
# Default build command (always use this)
cargo build --target x86_64-unknown-linux-musl --no-default-features --features rustls-backend
# Output: target/x86_64-unknown-linux-musl/debug/clay

cargo build --features webview-gui   # Build with webview GUI client
cargo test                           # Run all tests
cargo test test_name                 # Run a single test
cargo clippy                         # Lint
cargo fmt                            # Format code
cargo fmt -- --check                 # Check formatting without changes
```

**Why musl:** glibc static builds cause SIGFPE crashes during DNS resolution. musl handles DNS properly in fully static binaries.

**Why rustls:** native-tls requires OpenSSL cross-compilation for musl. rustls is pure Rust.

### Other Platforms

- **Termux (Android):** `cargo build --no-default-features --features rustls-backend` (no musl target needed). Hot reload and TLS proxy unavailable on Android.
- **macOS:** `cargo build --no-default-features --features rustls-backend` (no musl target needed). Universal binary via `lipo` combining x86_64 and aarch64 builds.
- **WebView GUI on Termux** requires X11 patches: run `./patches/apply-patches.sh` first.

## Architecture

Clay is a terminal MUD client built with **ratatui/crossterm** (TUI) and **tokio** (async networking). ~59K lines of Rust in a single crate.

### Core Structs

- **App** (`main.rs`): Central state — worlds list, current world index, input area, spell checker, settings, popup manager, keybindings, WebSocket server
- **World** (`main.rs`): Per-connection state — output buffer, scroll position, connection, unseen lines, pause state, settings, log handle, prompt
- **InputArea** (`input.rs`): Input buffer with viewport scrolling, command history, cursor position, kill ring
- **TfEngine** (`tf/mod.rs`): TinyFugue compatibility layer — variable storage, macro registry, command processing

### Module Map

| Area | Files | Purpose |
|------|-------|---------|
| **Core** | `main.rs` (25K), `daemon.rs`, `persistence.rs` | App struct, TUI loop, connections, hot reload, settings I/O |
| **Input/Display** | `input.rs`, `encoding.rs`, `spell.rs`, `util.rs` | Input handling, character encoding (UTF-8/Latin1/Fansi), word wrapping |
| **TF Engine** | `tf/` (11 files, ~11K) | TinyFugue commands, expressions, macros/triggers, control flow, hooks |
| **Networking** | `websocket.rs`, `http.rs`, `telnet.rs` | WebSocket server, HTTP/HTTPS server, telnet protocol |
| **Popups** | `popup/mod.rs`, `popup/console_renderer.rs`, `popup/definitions/` | Unified popup system with field types and layout engine |
| **Actions** | `actions.rs` | Pattern-matching triggers with command execution |
| **GUI** | `webview_gui.rs` | WebView GUI client (wry/tao) |
| **Web UI** | `web/index.html`, `web/style.css`, `web/app.js` | Browser-based client |
| **Theme** | `theme.rs`, `encoding.rs` | GUI/web themes (`ThemeColors` in theme.rs), console themes (`Theme` enum in encoding.rs) |
| **Tests** | `testharness.rs`, `testserver.rs` | Integration test framework and mock MUD server |

### Key Patterns

**Async architecture:** One tokio reader task per world reads raw bytes via `StreamReader`, sends `AppEvent::ServerData(world_idx, bytes)`. Writer tasks receive commands via channels. Main loop handles terminal events, decodes bytes, routes to worlds.

**Multi-world:** Each world has independent output buffer, scroll position, connection, unseen count. `unseen_lines` tracks activity for non-current worlds. WebSocket broadcasts keep all clients in sync.

**Hot reload (`/reload`):** Saves full state, clears FD_CLOEXEC on socket FDs, calls `exec()` to replace process with new binary. TCP connections survive. TLS connections require the TLS proxy feature (forked child process relays TLS↔Unix socket).

**More-mode pausing:** After `output_height - 2` new lines since last input, incoming lines queue to `pending_lines` instead of `output_lines`. Tab releases one screenful, Escape+j releases all.

**Line buffering:** `find_safe_split_point()` prevents splitting ANSI escape sequences and telnet commands across TCP reads. Partial lines (no trailing newline) display immediately and update in-place when completed.

**Popup system:** Unified `PopupManager` with declarative field definitions. Console rendering in `popup/console_renderer.rs`. Each popup defined in `popup/definitions/`.

**TF engine:** Commands work with both `/` and `#` prefixes. Parser in `tf/parser.rs` routes to builtins. Expression evaluator supports arithmetic, string functions, regex. Macros can be triggers (`-t"pattern"`), hooks (`-hCONNECT`), or key bindings (`-b"key"`).

**WebSocket protocol:** JSON messages over WebSocket. Clients authenticate with SHA-256 hashed password, then receive `InitialState`. All MUD output broadcast as `ServerData`. Cross-interface sync via `UnseenCleared`, `UnseenUpdate`, `ActivityUpdate` messages.

### Data Files

- `~/.clay.dat` — Settings (INI format with `[global]` and `[world:name]` sections)
- `~/.clay.reload` — Temporary hot reload state
- `~/clay.theme.dat` — Theme colors (INI format)
- `~/.clay.key.dat` — Keyboard bindings (INI, only non-default bindings)
- `~/.clay_media_cache/` — Cached GMCP media files

### Testing

Unit tests are inline in source files (`#[cfg(test)]` modules). Integration tests use a custom async harness:

- `testharness.rs`: `run_test_scenario(config, actions) -> Vec<TestEvent>` — creates App, spawns reader tasks, processes telnet, captures state changes
- `testserver.rs`: Mock MUD server with scripted scenarios (SendLine, SendGA, WaitForData, etc.)
- Key test event types: `Connected`, `TextReceived`, `MoreTriggered`, `AutoLoginSent`, `PromptReceived`
- `TestAction` enum for mid-scenario actions: `SwitchWorld`, `TabRelease`, `AssertState`, etc.

### /release Skill

Automated multi-platform build and GitHub release. Invoke with `/release [version]`. Skill files in `.claude/skills/release/`.

## Detailed Reference Docs

For detailed feature documentation, commands, and protocol specs, see `reference/`:
- `reference/commands.md` — Keyboard controls, client commands, popup controls, configurable keybindings
- `reference/tf-engine.md` — TF commands, variables, expressions, macros, hooks, control flow
- `reference/features.md` — Screen layout, encoding, actions, auto-login, spell check, popups, ANSI music, GMCP
- `reference/networking.md` — TLS, telnet protocol, WebSocket server/protocol, HTTP/HTTPS, hot reload details, GUI/remote/grep clients
