# TLS Proxy

The TLS Proxy feature allows TLS/SSL connections to be preserved across hot reloads.

## The Problem

TLS encryption state includes:
- Session keys
- Initialization vectors (IVs)
- Sequence numbers
- Other cryptographic state

This state exists only in process memory. When `exec()` replaces the process during hot reload, this memory is destroyed - even though the underlying TCP socket can survive.

Without TLS Proxy, TLS connections must be manually reconnected after every reload.

## The Solution

A forked child process handles TLS, communicating with the main process via Unix socket:

    MUD Server <--TLS--> TLS Proxy (child) <--Unix Socket--> Main Client

The proxy process:
- Survives `exec()` (separate process)
- Maintains TLS state
- Relays data between TLS connection and Unix socket
- Reconnects to main client after reload

## Configuration

### Enable TLS Proxy

1. Open `/setup`
2. Toggle "TLS Proxy" to On

### Behavior

When enabled:
- New TLS connections spawn a proxy process
- Proxy handles TLS termination
- Main client communicates via Unix socket
- On reload, new main process reconnects to existing proxies

## How It Works

### On TLS Connect

1. Main client forks a child process
2. Child establishes TLS connection to MUD
3. Child creates Unix socket at `/tmp/clay-tls-<pid>-<world_name>.sock`
4. Main client connects to Unix socket
5. Child relays: TLS ↔ Unix socket

### On Hot Reload

1. Main client saves proxy PID and socket path per world
2. `exec()` replaces main process
3. New main process reads saved proxy info
4. Reconnects to existing proxy via Unix socket
5. TLS connection continues uninterrupted

### On Disconnect

1. Main client closes Unix socket connection
2. Proxy detects disconnect
3. Proxy closes TLS connection and exits

## Implementation Details

### Functions

| Function | Purpose |
|----------|---------|
| `spawn_tls_proxy()` | Forks child, establishes TLS |
| `run_tls_proxy()` | Child main loop (relay) |

### Stream Types

```rust
enum StreamReader {
    Plain(TcpStream),
    Tls(TlsStream),
    Proxy(UnixStream),  // When using TLS proxy
}
```

### Saved State

During reload, per-world:
- `proxy_pid`: Process ID of proxy child
- `proxy_socket_path`: Path to Unix socket

## Socket Path

Format: `/tmp/clay-tls-<main_pid>-<world_name>.sock`

Example: `/tmp/clay-tls-12345-MyMUD.sock`

## Health Monitoring

The main client monitors proxy health:
- Detects if proxy process dies
- Marks world as disconnected on proxy death
- Cleans up Unix socket

## Fallback Behavior

If proxy spawn fails:
- Falls back to direct TLS connection
- Connection works but won't survive reload
- Warning logged

## When to Use

### Enable TLS Proxy

- You use TLS connections AND
- You want to preserve them across hot reload AND
- You reload frequently (development, automated deploys)

### Skip TLS Proxy

- Only use non-TLS connections
- Rarely use hot reload
- Minimal resource usage is priority
- Running on Termux (not available)

## Resource Usage

Each TLS proxy uses:
- One child process
- One Unix socket
- One TLS connection
- Minimal memory (relay only)

Proxies are lightweight but add process count.

## Platform Support

| Platform | TLS Proxy Support |
|----------|-------------------|
| Linux | Full |
| macOS | Full |
| Windows (WSL) | Full |
| Termux/Android | Not available |

Termux doesn't support TLS Proxy because:
- `exec()` is limited on Android
- Signal handling is restricted

## Troubleshooting

### TLS Connection Lost After Reload

1. Verify TLS Proxy is enabled in `/setup`
2. Check proxy process is running: `ps aux | grep clay`
3. Verify Unix socket exists: `ls /tmp/clay-tls-*`

### Proxy Not Spawning

1. Check disk space for socket file
2. Verify /tmp is writable
3. Check process limit not reached

### Socket Permission Errors

1. Check /tmp permissions
2. Verify socket file is accessible
3. May need to clean up stale sockets

### Zombie Proxy Processes

If proxy processes remain after Clay exits:
```bash
pkill -f "clay.*tls.*proxy"
```

Or clean up sockets:
```bash
rm /tmp/clay-tls-*.sock
```

## Example Session

```bash
# Start Clay
./clay

# Connect to TLS world
/worlds -e MyMUD
# Enable Use SSL, save

# Enable TLS Proxy
/setup
# Enable TLS Proxy, save

# Connect
/worlds MyMUD

# Make code changes, rebuild
# (in another terminal)
cargo build

# Reload - TLS connection preserved!
/reload
```

\newpage

