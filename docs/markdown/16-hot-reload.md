# Hot Reload

Clay supports hot reloading - replacing the running binary with a new version while preserving active connections.

## How It Works

The hot reload process:

1. **Save state**: Complete application state is serialized
   - Output buffers and pending lines
   - Scroll positions
   - Per-world settings (encoding, auto-login type, etc.)
   - Connection file descriptors

2. **Prepare sockets**: FD_CLOEXEC flag is cleared on socket file descriptors so they survive exec

3. **Execute**: `exec()` replaces the current process with the new binary

4. **Restore state**: New process detects reload mode and restores everything

5. **Reconstruct connections**: TCP sockets are rebuilt from preserved file descriptors

6. **Cleanup**: Inconsistent states are fixed
   - Worlds without working command channels marked disconnected
   - Pending lines cleared for disconnected worlds
   - Pause state cleared for disconnected worlds

## Triggering Reload

### /reload Command

```
/reload
```

### Keyboard Shortcut

Press `Ctrl+R`

### External Signal

Send SIGUSR1 to the process:

```bash
kill -USR1 $(pgrep clay)
```

Useful for:
- Automated deployment
- Scripts
- CI/CD pipelines

## Updated Binary Detection

On Linux, when you rebuild Clay while it's running:
- `/proc/self/exe` shows the path with " (deleted)" suffix
- The reload logic strips this suffix to find the new binary

This allows seamless workflow:
1. Make code changes
2. Run `cargo build`
3. Type `/reload` or press `Ctrl+R`
4. New code is active, connections preserved

## Limitations

### TLS/SSL Connections

**Without TLS Proxy:**
- TLS connections cannot be preserved
- TLS state (session keys, IVs, sequence numbers) exists only in process memory
- `exec()` destroys this state even though the TCP socket survives
- TLS worlds will need manual reconnection after reload

**With TLS Proxy:**
- Enable "TLS Proxy" in `/setup`
- TLS connections are preserved across reload
- See **TLS Proxy** chapter for details

### State Compatibility

The new binary must be compatible with the saved state format. Major version changes may break reload compatibility.

### Auto-Login

Restored connections have auto-login disabled:
- Prevents duplicate login attempts
- Only fresh connections trigger auto-login

## Message Suppression

During reload, success messages are suppressed to reduce noise:
- WebSocket/HTTP/HTTPS server startup (only shown on failure)
- Binary path message (only shown on failure)

Warnings and errors are always shown.

## Use Cases

### Apply Code Changes

Develop and test without losing sessions:

```bash
# Terminal 1: Running Clay
./clay

# Terminal 2: Make changes
vim src/main.rs
cargo build

# Terminal 1: Reload
/reload
# Changes active, still connected!
```

### Deploy Bug Fixes

Fix a bug without disconnecting users:

1. Build new binary
2. Send SIGUSR1: `kill -USR1 $(pgrep clay)`
3. Fix is live

### Configuration Changes

Some settings require reload to take effect. Instead of restarting:

```
/reload
```

## State File

During reload, state is temporarily saved to `~/.clay.reload`:
- Contains serialized application state
- Socket file descriptors
- World configurations
- Output buffers

This file is:
- Created during reload
- Read by the new process
- Automatically cleaned up

## Reload vs Restart

| Aspect | Reload | Restart |
|--------|--------|---------|
| TCP connections | Preserved | Lost |
| TLS connections | Requires TLS Proxy | Lost |
| Output history | Preserved | Lost |
| Scroll position | Preserved | Reset |
| Settings | Preserved | Reloaded from file |
| Memory usage | Cleaned | Fresh start |

## Troubleshooting

### Reload Fails

1. Check binary exists and is executable
2. Verify sufficient disk space for state file
3. Check file permissions on ~/.clay.reload

### Connections Lost After Reload

1. For TLS: Enable TLS Proxy
2. For TCP: Check network didn't drop during reload
3. Verify world cleanup didn't mark healthy connections as dead

### State Incompatibility

If reload fails due to state format changes:
1. Disconnect all worlds
2. Save important settings manually
3. Restart normally
4. Reconnect worlds

### SIGUSR1 Not Working

1. Verify correct PID: `pgrep clay`
2. Check signal permissions
3. Ensure Clay is the foreground process (not backgrounded with wrong signal handling)

## Technical Details

### State Serialization

State is serialized using a binary format including:
- Version marker for compatibility
- Output line vectors
- Pending line vectors
- Scroll offsets
- World configurations
- Socket file descriptors (as integers)

### Socket Preservation

Before exec():
```rust
// Clear close-on-exec flag
fcntl(fd, F_SETFD, 0);
```

After exec():
```rust
// Reconstruct socket from fd
let socket = TcpStream::from_raw_fd(fd);
```

### Process Replacement

Uses `execve()` to replace the process image:
- Same PID maintained
- Same parent process
- File descriptors preserved (with cleared FD_CLOEXEC)
- Memory completely replaced

\newpage

