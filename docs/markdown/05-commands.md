# Commands

All Clay commands start with `/`. These are client commands, not sent to the MUD server.

## General Commands

### /help
Opens the help popup with quick reference information.

```
/help
```

**Controls in help popup:**
- `Up/Down` - Scroll content
- `PageUp/PageDown` - Scroll faster
- `Enter` or `Esc` - Close popup

### /quit
Exit Clay and disconnect all worlds.

```
/quit
```

You can also press `Ctrl+C` twice within 15 seconds to quit.

## World Management

### /worlds
Open the World Selector popup or manage worlds.

```
/worlds                 # Open world selector popup
/worlds <name>          # Connect to world (create if doesn't exist)
/worlds -e [name]       # Edit world settings (current if no name)
/worlds -l <name>       # Connect without auto-login
```

**World Selector Controls:**
- `Up/Down` - Navigate list
- `Enter` - Connect to selected world
- `A` - Add new world
- `E` - Edit selected world
- `Tab` - Cycle through buttons
- Type to filter worlds

### /connections (or /l)
List all connected worlds in a table format.

```
/connections
/l
```

**Output columns:**

| Column | Description |
|--------|-------------|
| World | World name (* = current) |
| Unseen | Count of unseen lines |
| LastSend | Time since last command sent |
| LastRecv | Time since last data received |
| LastNOP | Time since last NOP keepalive |
| NextNOP | Time until next NOP |

### /disconnect (or /dc)
Disconnect the current world and close its log file.

```
/disconnect
/dc
```

## Sending Commands

### /send
Send text to one or more worlds.

```
/send <text>                # Send to current world
/send -w<world> <text>      # Send to specific world
/send -W <text>             # Send to ALL connected worlds
/send -n <text>             # Send without newline (no CR/LF)
```

**Examples:**
```
/send look                   # Send "look" to current world
/send -wMyMUD say hello     # Send "say hello" to MyMUD
/send -W ooc I'm here!      # Broadcast to all worlds
```

## Settings Commands

### /setup
Open the Global Settings popup.

```
/setup
```

**Available settings:**
- More mode - Enable/disable more-style pausing
- Spell check - Enable/disable spell checking
- Temp convert - Auto-convert temperatures in input
- World Switching - Cycling behavior (Unseen First, Alphabetical)
- Show tags - Show/hide MUD tags (also F2)
- Input height - Default input area height (1-15)
- Console Theme - Dark or Light
- GUI Theme - Dark or Light
- TLS Proxy - Enable TLS connection preservation
- ANSI Music - Enable music playback

### /web
Open Web Settings popup for HTTP/WebSocket configuration.

```
/web
```

See the **Web Interface** chapter for details.

### /actions
Open the Actions editor to create triggers.

```
/actions
```

See the **Actions** chapter for details.

## Utility Commands

### /reload
Hot reload the client with a new binary while preserving connections.

```
/reload
```

Also triggered by:
- `Ctrl+R` keyboard shortcut
- `SIGUSR1` signal (`kill -USR1 $(pgrep clay)`)

See the **Hot Reload** chapter for details.

### /testmusic
Play a test ANSI music sequence (C-D-E-F-G) to verify audio.

```
/testmusic
```

Requires ANSI Music enabled in `/setup` and a connected web/GUI client.

### /notify
Send a notification to the Android app.

```
/notify <message>
```

Useful in action commands:
```
/notify Someone is paging you!
```

## Command Completion

When input starts with `/`, press `Tab` to cycle through matching commands:

- Matches internal commands (`/help`, `/disconnect`, etc.)
- Matches manual actions (actions with empty patterns)
- Case-insensitive matching
- Arguments after the command are preserved

**Example:**
```
/wo[Tab]        -> /worlds
/worlds -e[Tab] -> cycles through other /w commands
```

\newpage

