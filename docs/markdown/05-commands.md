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

## Telnet Protocol Commands

See the **Telnet Features** chapter for background on MSSP/MSDP/GMCP.

### /mssp
Show the current world's MSSP (MUD Server Status Protocol) data — name/value pairs the
server volunteers about itself (player count, uptime, genre, and so on).

```
/mssp
```

Says so if the server hasn't sent any (MSSP not negotiated on this connection, or
negotiated but nothing received yet).

### /msdp
Send an outbound MSDP (MUD Server Data Protocol) command to the current world.

```
/msdp <LIST|REPORT|UNREPORT|SEND> [target]
```

The verb is case-insensitive. A target is optional and, when given, is joined back into a
single value (e.g. `/msdp REPORT HEALTH MANA` sends `HEALTH MANA` as one target).

**Examples:**
```
/msdp LIST REPORTABLE_VARIABLES
/msdp REPORT HEALTH
```

Refuses with a message if MSDP was never negotiated on this connection.

Clay already sends `LIST REPORTABLE_VARIABLES` and reports back everything the server names
in its reply as soon as MSDP negotiates (see the **Telnet Features** chapter), so `/msdp
REPORT` by hand is mainly for a variable the automatic pass missed, or a server that doesn't
answer `LIST`.

### /stats
Show the current world's status display — every stat derived from GMCP `Char.*` packages and
MSDP variables received so far. Recognized vitals (health, mana, movement, experience, level)
come first, then everything else alphabetically. A value paired with its maximum by name
shape (`hp`/`maxhp`, `HEALTH`/`HEALTH_MAX`, and similar) renders as one current/maximum gauge
entry; anything else renders as a plain labelled value — unrecognized fields are always shown
rather than dropped, since there is no fixed schema to match them against.

```
/stats
```

Says so if the world hasn't sent any of this data yet. `/stats` only displays what has
already arrived — it doesn't request anything itself. The same data drives the console's
one-line status display above the separator bar and the collapsible status panel on web/GUI/
Android (see the **Interface Overview** and **Web Interface** chapters); both appear
automatically the moment any data arrives and disappear again on disconnect, with nothing to
configure. Whether anything ever appears depends entirely on the MUD: GMCP `Char.*` and MSDP
are common on Diku-family servers, but MOOs and MUSHes generally implement neither, so
`/stats` — and the status line/panel — correctly stay empty there.

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

