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

### The build script

`./build.sh` builds a release binary with whatever this machine supports. It adds the
native GUI (`native-gui`) where GPUI's libraries and a Rust 1.98 toolchain are found, and
the WebView GUI (`webview-gui`) only where the native one can't be built - Termux, a
machine without those libraries, or a native build that fails - so a desktop build
carries one GUI, not two. Audio and the SSH transport come along where they can, and a
Linux machine with none of the GUI or audio libraries gets the static musl build below.

### Linux (Recommended: musl build)

The musl build produces a fully static binary that works on any Linux system regardless of glibc version:

```bash
# Install musl target (one-time setup)
rustup target add x86_64-unknown-linux-musl

# Build
cargo build --target x86_64-unknown-linux-musl \
    --no-default-features --features ssh-transport,shell-worlds

# Binary location
./target/x86_64-unknown-linux-musl/debug/clay
```

**Why musl instead of glibc?**

- glibc static builds cause SIGFPE crashes during DNS resolution
- glibc's NSS requires dynamic loading even in static builds
- musl handles DNS resolution properly in fully static binaries

**TLS is rustls, always.** It is pure Rust, so it needs no OpenSSL and builds the same
on every platform, and Clay pins certificates itself (trust on first use) rather than
asking the operating system's trust store. `rustls-backend` is still accepted as a
feature name, doing nothing, so older build commands keep working.

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
cargo build --no-default-features --features ssh-transport,shell-worlds

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
cargo build --no-default-features --features ssh-transport,shell-worlds

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
| `rustls-backend` | Accepted and ignored: TLS is always rustls now |
| `native-gui` | Build the native GUI (GPUI; needs Rust 1.98 or newer and a display; `--gui`) |
| `webview-gui` | Build the WebView GUI client (requires display; `--web-gui`, and `--gui` in a build without `native-gui`) |
| `native-audio` | ANSI music / MSP sound playback via rodio (on by default) |
| `ssh-transport` | SSH-tunneled `--console`/`--gui`/`--web-gui` and Android's SSH proxy (on by default) |
| `shell-worlds` | Shell worlds, a world that is a local terminal (on by default; in every release build except the Android app) |
| `multiuser` | The multiuser server, the `clay_multi` binary (off by default; in no release build). Never together with `shell-worlds` or `web-front` |
| `web-front` | The web gateway, the `clay_web` binary (off by default; in no release build). Never together with `shell-worlds` or `multiuser` |

`build.sh` builds three binaries into the repository directory: `./clay` (with shell
worlds), `./clay_multi` and `./clay_web`. `./build.sh --only clay|multi|web` builds one of
them, and `./build.sh --no-shell-worlds` leaves shell worlds out of `./clay`. With cargo, a
build that names `--no-default-features` must add `shell-worlds` back itself; the servers
are `cargo build --release --no-default-features --features multiuser,ssh-transport` and
`cargo build --release --no-default-features --features web-front,ssh-transport` (cargo
names every binary `clay`; build.sh builds the servers in their own `target/servers`
directory and copies them under their own names, so cargo's usual `target/release/clay`
is always the client).

A build without shell worlds still loads, shows and saves a shell world from your
settings; connecting it says the build has none, and the world editor offers no Shell type.
A server build started with no options runs its server; `clay --multiuser` or
`clay --web-front` in a build without that server says which feature it needs.

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

With no flag, a binary that has a GUI opens it wherever a window can be shown (a desktop
session; on Linux, a display server listening), and the console otherwise - in a terminal
reached over SSH, say. `--gui` and `--console` choose for you; see "With no flag" in the
GUI chapter.

## Command-Line Options

| Option | Description |
|--------|-------------|
| `-v`, `--version` | Show version and exit |
| `--conf=<path>` | Use alternate config file instead of `~/.clay/settings.dat` |
| `--console[=host[:port]]` | Console (TUI) mode; with an address, connects to a running Clay instance instead of starting a local one (default port: 9000) |
| `--gui[=host[:port]]` | GUI mode: the native GUI (`native-gui` feature; a build without it opens the WebView GUI instead); same local-vs-remote distinction as `--console` |
| `--web-gui[=host[:port]]` | WebView GUI mode (requires the `webview-gui` feature); same local-vs-remote distinction as `--console` |
| `--ssh` | Tunnel `--console=`/`--gui=`/`--web-gui=` through SSH instead of connecting directly (requires the `ssh-transport` feature) |
| `-D` | Run as a headless daemon server |
| `--multiuser` | Run the multiuser server (the `clay_multi` build runs it with no options too; see the Multiuser Server chapter) |
| `--web-front` | Run the web gateway (the `clay_web` build runs it with no options too; see the Web Gateway chapter) |
| `--local-server` | Run headless, loopback-only, for an embedding client (e.g. the Android app); with `CLAY_LOCAL_SERVER_LAN=1` in the environment it listens on every interface (the app's Allow Other Devices) |
| `--port=<N>` | Override the listen port (used with `--local-server`) |
| `--upgrade` | Download the newest release from GitHub, install it over this binary and exit, without starting the console or a GUI (`/update`'s install, from the command line). A Clay already running from the same file keeps its version until it restarts or `/reload`s. Not on Termux |

Remote connections always use `--console=host:port`, `--gui=host:port` or `--web-gui=host:port` — there is
no separate flag for it.

**Example using alternate config:**

```bash
./clay --conf=/path/to/my-config.dat
```

\newpage

