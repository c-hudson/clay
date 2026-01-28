# Multi-World System

Clay supports multiple simultaneous MUD connections, each with independent state and settings.

## Understanding Worlds

Each world has:
- Independent output buffer (unlimited scrollback)
- Independent scroll position
- Independent connection state
- Per-world settings (encoding, auto-login, logging)
- Unseen line counter for background activity

## World Switching

### Active World Cycling (Up/Down)

The `Up` and `Down` arrow keys cycle through **active** worlds:
- Connected worlds
- Disconnected worlds with unseen output

Disconnected worlds without unseen lines are skipped.

### All World Cycling (Shift+Up/Down)

`Shift+Up` and `Shift+Down` cycle through ALL worlds, including disconnected ones without unseen output.

### Activity-Based Switching (Escape+w or Alt+w)

Switches to the world with activity, using this priority:
1. World with oldest pending lines (paused output)
2. World with unseen output (oldest first)
3. Previous world

### World Switching Mode

Configure in `/setup`:

**Unseen First** (default):
- If any OTHER world has unseen output, switch to the one that received unseen output first
- Otherwise, switch alphabetically

**Alphabetical**:
- Always switch to the alphabetically next world
- Wraps from last to first

## Activity Indicators

### Separator Bar

The separator bar shows activity at a glance:

```
More: 1234 🟢 CurrentWorld___(Activity: 2)_____________14:30
```

- **More: XXXX**: Pending lines when current world is paused
- **Hist: XXXX**: Lines scrolled back in history
- **🟢 WorldName**: Currently viewing this connected world
- **(Activity: 2)**: Two OTHER worlds have unseen output

### Unseen Line Tracking

When output arrives for a non-current world:
- `unseen_lines` counter increments
- `first_unseen_at` timestamp is set (for "Unseen First" mode)

When you switch to a world:
- Its unseen count is cleared
- All clients are notified (console, web, GUI stay in sync)

## Managing Connections

### Listing Connections

Use `/connections` or `/l` to see all connected worlds:

```
/connections
```

Output:
```
World      Unseen  LastSend  LastRecv  LastNOP  NextNOP
*MyMUD            1m 30s    5s        4m 30s   30s
 OtherMUD  15     10m       30s       5m       idle
```

Columns:
- `*` marks the current world
- Unseen: Count of unseen lines (empty if 0)
- LastSend: Time since last command you sent
- LastRecv: Time since last data received
- LastNOP: Time since last keepalive sent
- NextNOP: Time until next keepalive

### Connecting to Worlds

```
/worlds MyMUD       # Switch to and connect
/worlds -l MyMUD    # Connect without auto-login
```

Or use the World Selector (`/worlds`) and press Enter.

### Disconnecting

```
/disconnect         # Disconnect current world
/dc                 # Short form
```

The world remains in your list for reconnection.

## Per-World Features

### Output Buffers

Each world maintains its own output buffer:
- Unlimited size (grows with available memory)
- Independent scroll position per world
- Independent more-mode pause state

### Prompts

Telnet prompts (GA/EOR) are per-world:
- Stored in the world's state
- Only displayed when viewing that world
- Cleared when you send a command

### Logging

Configure per-world in the World Editor:
- Log file opened on connect (append mode)
- All received output written to log
- Log file closed on disconnect

### Encoding

Each world can have different character encoding:
- UTF-8 (default)
- Latin1 (ISO-8859-1)
- Fansi (CP437-like)

## Cross-Interface Sync

World state syncs across all interfaces:

| Event | Console | Web | GUI |
|-------|---------|-----|-----|
| Output arrives | Shows in buffer | Broadcast | Broadcast |
| World switched | Clears unseen | Notified | Notified |
| Connect/Disconnect | Updates status | Notified | Notified |

Each interface can independently:
- View different worlds
- Have different scroll positions
- Be at different pause states

But they all see the same underlying data.

## Tips for Multi-World Usage

1. **Use activity switching**: `Escape+w` quickly jumps to worlds that need attention

2. **Watch the Activity indicator**: The separator bar shows how many worlds have unseen output

3. **Configure World Switching mode**:
   - "Unseen First" if you prioritize responding to activity
   - "Alphabetical" for predictable navigation

4. **Use world-specific actions**: Set the World field in actions to avoid triggers firing in the wrong context

5. **Name worlds clearly**: Names are used for navigation and action filtering

\newpage

