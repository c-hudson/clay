# Installation

## Pre-built Binaries

Pre-built binaries are available for common platforms. Download the appropriate binary for your system and make it executable:

```bash
# Linux x86_64 (static musl build - works on any Linux)
chmod +x clay-linux-x86_64-musl
./clay-linux-x86_64-musl

# Linux ARM64 (Termux)
chmod +x clay-linux-aarch64
./clay-linux-aarch64
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
    --no-default-features --features rustls-backend

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

### Linux with GUI Support

To build with the remote GUI client feature:

```bash
# Install dependencies (Debian/Ubuntu)
sudo apt install libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev

# Build with GUI
cargo build --features remote-gui

# Build with GUI and audio support
sudo apt install libasound2-dev
cargo build --features remote-gui-audio
```

### macOS

macOS builds use the native toolchain (no musl needed):

```bash
# Terminal client only
cargo build --no-default-features --features rustls-backend

# With GUI support
cargo build --features remote-gui

# With GUI and audio
cargo build --features remote-gui-audio

# Binary location
./target/debug/clay
```

Works on both Intel (x86_64) and Apple Silicon (aarch64) Macs.

### Termux (Android)

```bash
# Install Rust in Termux
pkg install rust

# Build (no GUI features available)
cargo build --no-default-features --features rustls-backend

# Binary location
./target/debug/clay
```

**Termux Limitations:**

- Hot reload not available (exec() limited on Android)
- TLS proxy not available
- Process suspension (Ctrl+Z) not available
- Remote GUI client not available

### Windows (via WSL)

Clay runs in Windows Subsystem for Linux:

```bash
# In WSL terminal
rustup target add x86_64-unknown-linux-musl
cargo build --target x86_64-unknown-linux-musl \
    --no-default-features --features rustls-backend
```

## Feature Flags

| Feature | Description |
|---------|-------------|
| `rustls-backend` | Use rustls for TLS (recommended for musl) |
| `native-tls-backend` | Use native TLS (requires OpenSSL) |
| `remote-gui` | Build remote GUI client (requires display) |
| `remote-gui-audio` | GUI with ANSI music playback |

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
| `--conf=<path>` | Use alternate config file instead of ~/.clay.dat |
| `--gui` | Start with GUI interface (requires remote-gui feature) |
| `--console` | Start with console interface (default) |
| `--remote=host:port` | Connect to remote Clay instance |
| `--console=host:port` | Console mode connecting to remote instance |
| `-D` | Daemon mode (background server only) |

**Example using alternate config:**

```bash
./clay --conf=/path/to/my-config.dat
```

\newpage

