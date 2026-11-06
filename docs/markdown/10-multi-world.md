# Multi-World System

Clay supports multiple simultaneous MUD connections, each with independent state and settings.

## Understanding Worlds

Each world has:
- Independent output buffer (unlimited scrollback)
- Independent scroll position
- Independent connection state
- Per-world settings (encoding, auto-login, logging)
- Unseen line counter for background activity

A world is usually a MUD, but it can also be a Slack or Discord channel (see
[Slack and Discord](21-chat-worlds.md)) or a local terminal (see
[Shell worlds](23-shell-worlds.md)) — a world whose Type is **Shell** runs your login
shell where the output would be, and all your other worlds stay one switch-key away.

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

## The Activity World

The Activity world shows every other world's new lines as they arrive, in one place. It
is meant for a split pane, so you can watch everything else while you play in one world:

```
/split act          # the Activity world above (or /split activity)
/world act          # go to it (or /world activity)
```

Each line starts with the name of the world it came from, then a space, then the line.
The name sits on a solid block of the **New Indicator** colour from Settings: green for
Background, the chosen colour for Custom, and yellow when the indicator has no colour
(None or Character). The colour is part of the line, so changing the setting changes
only lines that arrive afterwards.

The Activity world is only for watching:

- It never pauses for more, and nothing it shows counts as activity, adds to More or
  takes a ▶ marker, in any world.
- Its lines aren't logged, archived or spoken, and they fire no triggers or hooks. They
  already did all of that in their own worlds.
- It shows only what the MUD or chat server sent. Gagged lines, blank lines, Clay's own
  messages and what you type aren't copied. A MUD tag (`[chan:]`) is left off, as the world itself
  shows it.
- Each interface has its own. It holds only the lines that arrived while that interface
  showed it, so it starts empty each time you go to it. It never shows lines from a world
  that interface is showing itself: with `8bit` in the bottom pane and Activity above it,
  nothing from `8bit` appears in Activity, and when you switch the bottom pane to `8bit`,
  the `8bit` lines already in Activity go. That includes another window of the same GUI.
  What other interfaces show (the console, another GUI, a phone) makes no difference.
- The switch-world keys and Escape+w never go to it, and it isn't in the World
  Selector, the tabs, Recent Worlds or `/connections`. Only `/world` and `/split` reach
  it by name.
- It can't be edited, connected, deleted or given notes. Text typed while it has the
  focus goes nowhere, as in any world that isn't connected.

Every interface has it: the console, the SSH console, the GUIs, the web page and the
Android app. A multiuser server has none. If you already have a world of your own named
Activity, that world keeps the name and there is no Activity world; a new world can't
take the name.

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
- Independent more-mode pause state, in every interface: the web/Android client and the
  native GUI page a world they aren't showing too, so switching to a world with a backlog
  shows one screenful and holds the rest under MORE, as the console does

### Prompts

Telnet prompts (GA/EOR) are per-world:
- Stored in the world's state
- Only displayed when viewing that world
- Cleared when you send a command

On a **MUD - Timed Prompt** world a prompt is also inferred from a line the server leaves
unfinished for longer than that world's Prompt wait; the wait is a per-world setting. The
line stays in the output either way (see [Telnet](14-telnet.md#timed-prompts-mud---timed-prompt-worlds)).

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

