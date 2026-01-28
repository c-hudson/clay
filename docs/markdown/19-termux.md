# Android/Termux

Clay compiles and runs on Termux, the Android terminal emulator, bringing MUD access to mobile devices.

## Installation

### Install Termux

Download Termux from F-Droid (recommended) or Google Play Store.

F-Droid: https://f-droid.org/packages/com.termux/

### Install Rust

```bash
pkg update
pkg install rust
```

### Build Clay

```bash
# Clone the repository
git clone https://github.com/your-repo/clay
cd clay

# Build (no GUI features)
cargo build --no-default-features --features rustls-backend

# Run
./target/debug/clay
```

### Pre-built Binary

If a pre-built ARM64 binary is available:

```bash
chmod +x clay-linux-aarch64
./clay-linux-aarch64
```

## Limitations

Some features are unavailable on Termux due to Android restrictions:

### Not Available

| Feature | Reason |
|---------|--------|
| Hot reload | `exec()` is limited on Android |
| TLS proxy | Requires `exec()` for reload |
| Process suspension (Ctrl+Z) | Signal handling restricted |
| Remote GUI client | No display server |
| SIGUSR1 reload trigger | Signal handling restricted |

### What Works

| Feature | Status |
|---------|--------|
| Core MUD client | Full |
| Multiple worlds | Full |
| TLS connections | Direct only (not via proxy) |
| More-mode pausing | Full |
| Actions/triggers | Full |
| Command history | Full |
| All TUI features | Full |
| Settings persistence | Full |
| WebSocket server | Full |
| Web interface | Full |

## Android App

Clay has a companion Android app that provides:
- Native Android interface
- Background WebSocket connection
- Push notifications via `/notify` command
- Foreground service for persistent connection

### Installing the App

The APK can be built from the `android/` directory in the repository.

### Notifications

Use `/notify` to send notifications:

```
/notify Someone is paging you!
```

Or in actions:
```
Name: page_alert
Pattern: *pages you*
Command: /notify Page from $1
```

### Foreground Service

When authenticated:
- Foreground service starts
- Shows "Connected to MUD server" notification
- Keeps WebSocket alive in background
- Allows receiving notifications

## Tips for Termux

### Storage Setup

```bash
# Allow access to shared storage
termux-setup-storage
```

### Keyboard

- Use Termux:Styling for better fonts
- Consider external keyboard for extended sessions
- Swipe left on keyboard for arrow keys

### Session Management

Since hot reload isn't available:
- Use Termux sessions for multiple views
- Consider tmux for session persistence:

```bash
pkg install tmux
tmux new -s clay
./clay
# Detach with Ctrl+B then D
# Reattach with: tmux attach -t clay
```

### Battery Optimization

- Disable battery optimization for Termux
- Settings → Apps → Termux → Battery → Unrestricted

### Wake Lock

Keep Termux running:
```bash
termux-wake-lock
./clay
# When done:
termux-wake-unlock
```

## Building Tips

### Memory Usage

Termux has limited memory. If build fails:
```bash
# Reduce parallel jobs
cargo build -j 1 --no-default-features --features rustls-backend
```

### Storage Space

Rust builds need significant space:
- Clean old builds: `cargo clean`
- Check space: `df -h`

### Build Time

Initial build takes a while on mobile:
- Leave device plugged in
- Disable sleep during build
- Consider building on PC and copying binary

## Troubleshooting

### Connection Issues

1. Check network connectivity
2. Verify hostname resolution works
3. Try IP address instead of hostname

### Display Issues

1. Check terminal size: `stty size`
2. Try different Termux font
3. Resize terminal and press Ctrl+L

### Performance

1. Close other apps
2. Reduce scrollback if memory is tight
3. Disable spell checking in `/setup`

### Build Failures

1. Update packages: `pkg update && pkg upgrade`
2. Check disk space
3. Try building with less parallelism: `cargo build -j 1`

### Settings Not Saving

1. Check Termux has storage permission
2. Verify ~/.clay.dat is writable
3. Check disk isn't full

## Recommended Setup

```bash
# One-time setup
pkg update
pkg install rust tmux
termux-setup-storage

# Clone and build
git clone https://github.com/your-repo/clay
cd clay
cargo build --no-default-features --features rustls-backend

# Run in tmux for persistence
tmux new -s clay
./target/debug/clay

# Later, reattach
tmux attach -t clay
```

\newpage

