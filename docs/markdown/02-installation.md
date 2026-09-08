# Installation

## Pre-built Binaries

Pre-built binaries are available for common platforms. Download the appropriate binary for your system and make it executable:

```bash
# Linux x86_64 (static musl build - works on any Linux)
chmod +x clay-linux-x86_64-musl
./clay-linux-x86_64-musl

# Termux ARM64, no GUI (works on any Android device via Termux)
chmod +x clay-termux-aarch64-nogui
./clay-termux-aarch64-nogui
```

## Building from Source

### Prerequisites

You need Rust installed. If you don't have it:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
```

### Linux (Recommended: musl build)

The musl build produces a fully static binary that works on any Linux system regardless of glibc version:

```bash
# Install musl target (one-time setup)
rustup target add x86_64-unknown-linux-musl

# Build
cargo build --target x86_64-unknown-linux-musl \
    --no-default-features --features rustls-backend,ssh-transport

# Binary location
./target/x86_64-unknown-linux-musl/debug/clay
```

**Why musl instead of glibc?**

- glibc static builds cause SIGFPE crashes during DNS resolution
- glibc's NSS requires dynamic loading even in static builds
- musl handles DNS resolution properly in fully static binaries

**Why rustls instead of native-tls?**

- native-tls requires OpenSSL, which needs cross-compilation setup for musl
- rustls is pure Rust and works seamlessly with musl builds

### Linux with WebView GUI

To build with the native WebView GUI client feature (`webview-gui` — wry/tao;
`native-audio` for ANSI music/MSP sound is on by default alongside it):

```bash
# Install dependencies (Debian/Ubuntu)
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libasound2-dev

# Build with GUI (audio included by default)
cargo build --features webview-gui
```

### macOS

macOS builds use the native toolchain (no musl needed):

```bash
# Terminal client only
cargo build --no-default-features --features rustls-backend,ssh-transport

# With WebView GUI (audio included by default, no extra deps)
cargo build --features webview-gui

# Binary location
./target/debug/clay
```

Works on both Intel (x86_64) and Apple Silicon (aarch64) Macs — the official release
ships a universal binary combining both via `lipo`.

### Termux (Android)

```bash
# Install Rust in Termux
pkg install rust

# Build TUI only (no GUI dependencies)
cargo build --no-default-features --features rustls-backend,ssh-transport

# Binary location
./target/debug/clay
```

A WebView build is also possible on Termux with
[Termux:X11](https://github.com/termux/termux-x11) and the patches in
`./patches/apply-patches.sh` — see the pre-built `clay-termux-aarch64` release binary.

**Termux Limitations (regardless of GUI):**

- Hot reload not available (`exec()` limited on Android)
- TLS proxy not available
- Process suspension (Ctrl+Z) not available

### Windows

Native build (MSVC) — no WSL required:

```bash
# Install Rust from https://rustup.rs
# Install Visual Studio Build Tools (MSVC)

# Build with WebView GUI (uses WebView2). Static CRT linking eliminates a
# vcruntime140.dll runtime dependency.
set RUSTFLAGS=-C target-feature=+crt-static
cargo build --release --features webview-gui
```

## Feature Flags

| Feature | Description |
|---------|-------------|
| `rustls-backend` | Use rustls for TLS (recommended for musl) |
| `native-tls-backend` | Use native TLS (requires OpenSSL) |
| `webview-gui` | Build the native WebView GUI client (requires display; `--gui`) |
| `native-audio` | ANSI music / MSP sound playback via rodio (on by default) |
| `ssh-transport` | SSH-tunneled `--console`/`--gui` and Android's SSH proxy (on by default) |

## Verifying the Installation

```bash
# Run Clay
./clay

# Check version
./clay --version

# Show help
./clay --help
```

On first run, Clay creates a settings file at `~/.clay.dat` and displays a colorful splash screen with quick command hints.

## Command-Line Options

| Option | Description |
|--------|-------------|
| `-v`, `--version` | Show version and exit |
| `--conf=<path>` | Use alternate config file instead of `~/.clay/settings.dat` |
| `--console[=host[:port]]` | Console (TUI) mode; with an address, connects to a running Clay instance instead of starting a local one (default port: 9000) |
| `--gui[=host[:port]]` | WebView GUI mode (requires the `webview-gui` feature); same local-vs-remote distinction as `--console` |
| `--ssh` | Tunnel `--console=`/`--gui=` through SSH instead of connecting directly (requires the `ssh-transport` feature) |
| `-D` | Run as a headless daemon server |
| `--multiuser` | Run as a multiuser server (separate config, one process serves several independent accounts) |
| `--local-server` | Run headless, loopback-only, for an embedding client (e.g. the Android app) |
| `--port=<N>` | Override the listen port (used with `--local-server`) |

Remote connections always use `--console=host:port` or `--gui=host:port` — there is
no separate flag for it.

**Example using alternate config:**

```bash
./clay --conf=/path/to/my-config.dat
```

\newpage

