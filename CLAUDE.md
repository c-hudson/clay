# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Critical Rules

**All UI changes must be reflected in ALL interfaces.** Any new field, option, or window added to one interface (console TUI, web, webview-GUI) must be added to all three. New world/settings fields must also be saved to `~/.clay/settings.dat` in `persistence.rs` and loaded on startup and `/reload`.

**World passwords are stored encrypted in `~/.clay/settings.dat` but sent as plaintext to authenticated WebSocket clients and displayed as readable text in all UI editors.** Do not hide or mask world passwords in the world editor — the encryption is for at-rest storage only. The `has_password` field mirrors whether the password is non-empty.

**Do not write to stdout/stderr once the TUI is initialized (no `println!`, `eprintln!`, `dbg!`).** The rule exists because such output corrupts the ratatui screen (scroll regions, separator bar) after it's been drawn. It does NOT apply before the TUI is initialized, nor in headless contexts that never draw a TUI — startup/config messages before the alternate screen is entered, the `-D` daemon and `--multiuser` server (both headless, including the interactive first-run wizard's operator output), and the panic hook during teardown may use `println!`/`eprintln!` normally. Once the TUI is live, use instead:
- `debug_log(true, msg)` for always-on logging (writes to `~/.clay/debug.log`)
- `debug_log(is_debug_enabled(), msg)` for user-toggled debug
- `output_debug_log(msg)` for output/seq debugging (writes to `~/.clay/output.debug.log`)
- `add_tf_output()` to display messages in the output area

Debug output interferes with the TUI and corrupts the terminal display *once the TUI is live* (see the exception above for pre-init/headless/panic contexts). When the TUI is running, instead:
- Use `debug_log(true, msg)` for always-on logging or `debug_log(is_debug_enabled(), msg)` for user-toggled debug (writes to `~/.clay/debug.log`)
- Use `output_debug_log(msg)` for output/seq debugging (writes to `~/.clay/output.debug.log`)
- Display messages in the output area using `add_tf_output()` or `add_output()`

## Build Commands

```bash
# Default build command (always use this)
cargo build --target x86_64-unknown-linux-musl --no-default-features --features rustls-backend
# Output: target/x86_64-unknown-linux-musl/debug/clay

cargo build --features webview-gui   # Build with webview GUI client
cargo test                           # Run all tests
cargo test test_name                 # Run a single test
cargo clippy                         # Lint
```

**Why musl:** glibc static builds cause SIGFPE crashes during DNS resolution. **Why rustls:** native-tls requires OpenSSL cross-compilation for musl.

### Cross-Platform Builds

- **Termux:** `cargo build --no-default-features --features rustls-backend` (no musl target). GUI requires X11 patches via `./patches/apply-patches.sh`. No hot reload, TLS proxy, or Ctrl+Z on Android.
- **macOS:** `cargo build --no-default-features --features rustls-backend` (no musl). Universal binary via `lipo` combining x86_64-apple-darwin and aarch64-apple-darwin targets.
- **Windows:** `set RUSTFLAGS=-C target-feature=+crt-static` then `cargo build --release --features webview-gui` (MSVC, not cross-compiled)

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
- `src/persistence.rs` - Settings save/load (`~/.clay/settings.dat` INI format)
- `src/daemon.rs` - Daemon/headless mode, background connection logic
- `src/keybindings.rs` - Configurable keyboard bindings, load/save `~/.clay/keybindings.dat`

**Networking:**
- `src/websocket.rs` - WebSocket server, message types, client management
- `src/http.rs` - HTTP/HTTPS web server (3 handler implementations: native-tls, rustls, plain)

**Theme:**
- `src/theme.rs` - ThemeColors (42 customizable color vars), ThemeFile for `~/.clay/theme.dat`. GUI/web only; console uses Theme enum from encoding.rs.

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

**Data Files** (all inside `~/.clay/` on Unix, `~/clay/` on Windows):
- `settings.dat` - Main settings (INI format with `[global]` and `[world:name]` sections)
- `secure.key` - Per-machine AES-256 encryption key (binary, 0600 permissions)
- `known_hosts.dat` - Trust-on-first-use TLS certificate pins (`host:port` -> hex SHA-256 of the end-entity cert DER), 0600 permissions. Written by `persistence::add_pin`/`replace_pin`, read by `persistence::get_pin`; enforced by `platform::danger_rustls::TofuVerifier` (rustls MUD/remote-console/WebView-proxy connections) and `platform::check_native_tls_peer_pin` (native-tls MUD path).
- `theme.dat` - Theme colors (INI format with `[theme:name]` sections)
- `keybindings.dat` - Keyboard bindings (INI, only non-default bindings saved)
- `multiuser.dat` - Multiuser server settings
- `scrollback.db` - SQLite long-term scrollback archive
- `cert.pem` / `key.pem` - Auto-generated TLS cert/key for `web_secure` mode
- `debug.log` - Debug logging (via `debug_log()`)
- `output.debug.log` - Output/seq debugging (via `output_debug_log()`)
- `remote.log` - Remote connection events (silent drops, gate/knock outcomes, bans, WebSocket auth attempts)
- `dump.log` - `/dump` debug state output
- `settings-audit.log` - Debug-mode-only audit trail of `[global]` settings changes: old→new values, source client (web/gui/console/android/local), and a backtrace (via `persistence::save_settings_with_source()`, only written when debug mode is on)
- `logs/<WorldName>.<YYYY-MM-DD>.log` - Per-world session logs (when log_enabled)
- `media/` - Downloaded media cache

### Key Design Patterns

- **Borrow checker with App**: Clone data out of App before mutable borrows (e.g., ThemeColors, world info for WebSocket)
- **WebSocket InitialState**: `build_initial_state()` sends only `output_lines` (NOT pending_lines) to avoid duplicates. Partial lines excluded from broadcasts.
- **Hot reload**: `exec()` replaces process, TCP socket fds preserved via cleared FD_CLOEXEC. TLS connections need proxy process. On Windows GUI, the HTTP listener socket handle is passed to the child process via `CLAY_HTTP_LISTENER` env var to avoid port zombie issues.
- **GUI mode**: Default on Windows/macOS (no `--gui` flag needed). Uses `run_master_webgui()` → `run_app_headless()`. Forces `web_secure=false` and `http_enabled=true`. WebView connects via `ws://127.0.0.1:<http_port>`.
- **`/window --grep`**: Opens a half-height WebView with `GREP_MODE` JS var injected. Client-side filtering — no server changes. Matches against displayed text (strips ANSI + MUD tags). Regex compiled once at startup.
- **Console rendering**: `render_output_crossterm()` bypasses ratatui for output area (raw crossterm for ANSI fidelity). Popups use ratatui.
- **Web content**: HTML/CSS/JS embedded via `include_str!()` in http.rs. Changes require rebuild + `/reload`.
- **TF control flow in macros**: Body split by `%;`, control flow blocks grouped by `split_body_preserving_control_flow()`. Plain text in loop bodies becomes `SendToMud`, queued in `engine.pending_commands`.
- **Async lock access**: Use `try_read()`/`try_write()` on tokio RwLock from sync code, never `blocking_read()`/`blocking_write()` inside async runtime.

### Connection Security

Incoming connections are gated in `src/http.rs` before anything is served. Design record: `SECURITY-ROADMAP.md`; user-facing summary: `SECURITY-NOTES.md`.

- **Stealth path (`web_path` setting, default `"clay"`)**: web UI served only under `/clay/...`. Any other path from a non-localhost client is silently dropped (connection closed, zero response bytes) and records a ban strike — except `/favicon.ico` and `/apple-touch-icon*`, which drop without a strike. Empty `web_path` = legacy mode (UI at `/`, 404s as before). Localhost always works at both `/` and `/clay/` (the GUI WebView depends on this). Routing decisions live in the pure `decide_route()`; `SecurityGate` carries the shared allow-list/auth-key/ban-list state into all three server variants.
- **Accept-time IP gate** (`gate_connection()`): when `websocket_allow_list` is non-empty, non-listed non-localhost IPs are dropped *before* the TLS peek — no handshake, no certificate, no redirect (`GATE-DROP`, deliberately not a ban strike, or the knock below could never get through). Allow-list membership never skips authentication.
- **CLAY-KNOCK v1**: in-band auth-key preamble on the same TCP connection, before TLS/HTTP. Client sends `C7 4C 41 59 01 00`; server replies `C7 4B` + 32 random bytes; client sends raw `SHA256(auth_key || challenge)`; server acks `C7 06`. First byte `0xC7` disambiguates from TLS (`0x16`) and HTTP (ASCII) in the existing first-byte peek. A knocked connection may **WebSocket-upgrade at any path but never fetch a page** (`KNOCK-HTTP-DENIED`), and still performs normal WS auth. Android implements it in `NativeWebSocket.java` (`KnockSocketFactory`/`KnockSocket`, with fallback for old servers). Multiuser has no auth key → knocks always fail there.
- **WS auth matrix**: no allow list → password or auth key from anywhere; allow list set → password only from listed/localhost/whitelisted addresses, everyone else must knock with the auth key.
- **Ban exemption (D6)**: once an allow list is configured, the accept-time gate already drops every non-listed IP before it can reach any probe-strike site — so a probe strike can only ever ban a *legitimate, allow-listed* caller, never a scanner. `SecurityGate::strike()` is the one chokepoint every probe-strike site calls; it never bans an IP that's localhost, runtime-whitelisted, or matches a *specific* allow-list entry (exact IP, IP wildcard, or hostname pattern). A bare `*` allow-list entry does **not** confer this exemption — `*` means "let everyone reach the UI," not "nobody can ever be banned." `redirect_http_to_https()` (the plain-HTTP-on-the-HTTPS-port handler) reuses `decide_route()` directly instead of a separate reachability check, so it can never drift out of sync with it again — that drift was the root cause of a bug where an allow-listed user typing `http://` instead of `https://` got banned after two tries. Failed WebSocket password auth is the one exception: it still bans, and still applies to allow-listed IPs, via `BanList::record_auth_failure()` (threshold 5, not 2 — see `SECURITY-ROADMAP.md` D6). A connection that already knocked skips the "not in allow list" WS strike too (it proved a valid key; banning it would lock it out of its own recovery path).
- **Debugging**: `~/.clay/remote.log` records `HTTP-DROP`, `GATE-DROP`, `GATE-TIMEOUT`, `TLS-ON-PLAIN` (ClientHello on a plain-HTTP server — logged, never struck), `KNOCK-OK`/`KNOCK-FAIL`/`KNOCK-BAD-MAGIC`, `KNOCK-HTTP-DENIED`, `WS-PATH-DROP` alongside the existing `BANNED`/`CONN-LIMIT`/`TLS-*` events. A silent drop is intentional — expect zero bytes, not an error page. `log_remote_event()` is a no-op under `#[cfg(test)]` — tests must never append to a real user's `~/.clay/remote.log`.

**Outbound TLS is pinned, not CA-verified (D7).** Every client-side TLS connection (MUD worlds, remote-console, WebView proxy, hot-reload proxy, `/connect`) uses `platform::danger_rustls::TofuVerifier` (rustls) or `platform::check_native_tls_peer_pin` (native-tls), not CA verification — Clay's own server and most MUDs are self-signed. Trust-on-first-use: pin `sha256(end_entity_DER)` in `~/.clay/known_hosts.dat` (`persistence::add_pin`/`get_pin`/`replace_pin`) silently on first sight; on a mismatch, **block** and surface old-vs-new fingerprint + a "trust new cert" action in all three UIs (`WsMessage::CertMismatch`/`TrustCertificate`, web `showCertMismatchDialog`, TUI `create_cert_mismatch_dialog`). **The signature-verification methods MUST do real verification** (delegate to `rustls::crypto`) — pinning the fingerprint alone is defeatable by replaying the public cert without its key. See `SECURITY-ROADMAP.md` D7.

**Other D7 invariants**: secret files go through `util::write_secret_file`/`secure_create_file`/`secure_append_file` (0600; `~/.clay` is 0700) — never plain `File::create` for anything holding a password/key/token. Static-secret comparisons use `util::constant_time_eq`. Multiuser handlers taking a client `world_index` must check `world.owner == username` (see `ConnectWorld`/`SwitchWorld` in `daemon.rs`). MUD text reaching the web client must be escaped — `app.js` `escapeHtml` (incl. quotes) + `sanitizeHtml` on output sinks, and any HTML-building helper (e.g. `convertDiscordEmojis`) must escape what it interpolates. GMCP media URLs are http/https-only with internal targets refused.

### /release Skill

Automated multi-platform build and GitHub release. Invoke with `/release [version]`. Skill files in `.claude/skills/release/`.

Skill files: `.claude/skills/release/SKILL.md` (instructions), `.claude/skills/release/machines.md` (machine details).
