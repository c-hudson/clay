# Troubleshooting

Common issues and their solutions.

## Connection Problems

### Can't Connect to MUD

**Symptoms:** Connection times out or refused

**Solutions:**
1. Verify hostname and port are correct
2. Check if MUD is online: `telnet hostname port`
3. Check firewall isn't blocking
4. Try IP address instead of hostname
5. Check if SSL is required but not enabled (or vice versa)

### Connection Drops

**Symptoms:** Disconnected after period of inactivity

**Solutions:**
1. Enable keepalive in World Settings
2. Try different keepalive type (NOP, Generic, Custom)
3. Check if MUD has short idle timeout
4. Verify network is stable

### SSL/TLS Handshake Fails

**Symptoms:** Connection refused or handshake error

**Solutions:**
1. Verify MUD actually supports SSL on that port
2. Check if MUD uses self-signed certificate (may need configuration)
3. Try without SSL to verify server is reachable
4. Update CA certificates: `sudo update-ca-certificates`

## Display Issues

### Garbled Output

**Symptoms:** Strange characters, wrong symbols

**Solutions:**
1. Check encoding in World Settings (try Latin1, Fansi)
2. Verify TERM environment variable is set correctly
3. Check terminal supports UTF-8
4. Press Ctrl+L to redraw

### Colors Wrong

**Symptoms:** Wrong colors or missing colors

**Solutions:**
1. Check TERM is set (e.g., `xterm-256color`)
2. Verify terminal supports colors
3. Try different theme in `/setup`
4. Check terminal color scheme

### Screen Corruption

**Symptoms:** Text overlapping, wrong positions

**Solutions:**
1. Press Ctrl+L to redraw
2. Resize terminal window
3. Check terminal size: `stty size`
4. Restart Clay if persistent

### Wide Characters Display Wrong

**Symptoms:** CJK characters, emoji misaligned

**Solutions:**
1. Ensure terminal uses monospace font with CJK support
2. Check font supports wide characters
3. Try different terminal emulator

## Input Problems

### Keys Not Working

**Symptoms:** Function keys, arrows don't work

**Solutions:**
1. Check TERM environment variable
2. Verify terminal sends correct escape sequences
3. Try different terminal emulator
4. Check for conflicting terminal shortcuts

### Input Lag

**Symptoms:** Typing feels slow

**Solutions:**
1. Check network latency to MUD
2. Disable spell checking in `/setup`
3. Verify not running in slow emulator

### Cursor Position Wrong

**Symptoms:** Cursor not where expected

**Solutions:**
1. Press Ctrl+L to redraw
2. Check for zero-width characters in input
3. Verify font is monospace

## WebSocket/Web Interface

### Can't Connect to Web Interface

**Symptoms:** Browser shows connection refused

**Solutions:**
1. Verify WebSocket server is enabled in `/web`
2. Check password is set
3. Verify correct port
4. Check firewall allows the port
5. For HTTPS, verify TLS certificate is valid

### Web Interface Shows "Disconnected"

**Symptoms:** Connected then immediately disconnects

**Solutions:**
1. Check WebSocket password is correct
2. Verify WebSocket port matches
3. Check for HTTPS/HTTP mismatch (ws:// vs wss://)
4. Look for browser console errors

### Authentication Fails

**Symptoms:** Password rejected

**Solutions:**
1. Verify password matches `/web` settings
2. Check caps lock
3. Try clearing browser cache
4. If using allow list, verify IP is in list

## Hot Reload

### Reload Fails

**Symptoms:** `/reload` causes crash or disconnect

**Solutions:**
1. Verify new binary exists
2. Check disk space for state file
3. Ensure ~/.clay.reload is writable
4. For TLS connections, enable TLS Proxy

### Connections Lost After Reload

**Symptoms:** Worlds disconnected after `/reload`

**Solutions:**
1. TLS connections: Enable TLS Proxy in `/setup`
2. TCP connections: Network may have dropped
3. Check reload cleaned up stale connections

### State Incompatible

**Symptoms:** Error about state version

**Solutions:**
1. Save important settings manually
2. Disconnect all worlds
3. Restart normally (don't reload)
4. Reconnect and reconfigure

## Performance

### High Memory Usage

**Symptoms:** Clay using lots of RAM

**Solutions:**
1. Reduce scrollback (code change required)
2. Disconnect unused worlds
3. Clear old output with Ctrl+L
4. Restart periodically

### High CPU Usage

**Symptoms:** Clay using CPU when idle

**Solutions:**
1. Check for runaway triggers
2. Disable ANSI music if not needed
3. Reduce web client refresh rate
4. Check for reconnection loops

### Slow Startup

**Symptoms:** Takes long to start

**Solutions:**
1. Check dictionary file size (for spell check)
2. Verify network DNS is fast
3. Disable auto-connect temporarily

## Settings

### Settings Not Saving

**Symptoms:** Changes lost on restart

**Solutions:**
1. Check ~/.clay.dat is writable
2. Verify disk space
3. Make sure to press Save in popups
4. Check file permissions

### Settings Corrupted

**Symptoms:** Error loading settings

**Solutions:**
1. Backup ~/.clay.dat
2. Delete file and restart (recreates with defaults)
3. Manually edit file to fix syntax

## Build Problems

### Musl Build Fails

**Symptoms:** Compilation errors with musl target

**Solutions:**
1. Install musl tools: `sudo apt install musl-tools`
2. Add target: `rustup target add x86_64-unknown-linux-musl`
3. Clean and rebuild: `cargo clean && cargo build`

### GUI Build Fails

**Symptoms:** remote-gui feature won't compile

**Solutions:**
1. Install X11 libs: `sudo apt install libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev`
2. For audio: `sudo apt install libasound2-dev`
3. Verify display server is running

### Missing Dependencies

**Symptoms:** Link errors

**Solutions:**
1. Use `--no-default-features --features rustls-backend` for minimal deps
2. Check platform-specific requirements
3. Update Rust: `rustup update`

## Recovery

### Complete Freeze

If Clay becomes unresponsive:
1. Try Ctrl+C twice (quit)
2. Send SIGTERM: `kill $(pgrep clay)`
3. Send SIGKILL as last resort: `kill -9 $(pgrep clay)`

### Lost All Settings

Restore from backup:
```bash
cp ~/.clay.dat.backup ~/.clay.dat
```

Or start fresh - Clay creates defaults on startup.

### Crash on Startup

1. Rename settings file: `mv ~/.clay.dat ~/.clay.dat.broken`
2. Start Clay (creates fresh settings)
3. Manually migrate important settings

\newpage

