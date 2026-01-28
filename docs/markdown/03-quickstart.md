# Quick Start

## First Launch

When you first start Clay, you'll see a colorful ASCII art splash screen:

![Clay Startup Screen](images/tui/startup.png)

The splash screen shows the Clay logo with the tagline "A 90s MUD client written today" and quick command hints.

## Creating Your First World

1. Type `/worlds` to open the World Selector popup:

![World Selector](images/tui/world-selector.png)

2. Press `A` or click "Add" to create a new world

3. Fill in the world settings:

![World Editor](images/tui/world-editor.png)

   - **World name**: A friendly name (e.g., "MyMUD")
   - **Hostname**: The server address (e.g., "mud.example.com")
   - **Port**: The server port (e.g., 4000)
   - **Use SSL**: Enable if the server supports TLS
   - Optional: **User** and **Password** for auto-login

4. Press `Enter` on "Connect" or press `O` to save and connect

## Connecting to a World

Once configured, there are several ways to connect:

```
/worlds MyMUD          # Connect to a world by name
/worlds -e MyMUD       # Edit a world's settings
/worlds -l MyMUD       # Connect without auto-login
```

Or use the World Selector (`/worlds`) and press Enter on your world.

## Basic Navigation

### Screen Layout

The interface has three main areas stacked vertically:

**Output Area** (top, largest)
: Displays MUD text with ANSI colors. Unlimited scrollback.

**Separator Bar** (one line)
: Shows status, world name, activity count, and time.

**Input Area** (bottom, 1-15 lines)
: Where you type commands. Shows server prompt if detected.

Example separator bar:

    More: 1234 * HeartOfGold     (Activity: 2)              14:30

### Separator Bar Components

| Position | Content | Description |
|----------|---------|-------------|
| Left | `More: XXXX` | Pending lines when paused |
| Left | `Hist: XXXX` | Lines scrolled back in history |
| Center-left | `* WorldName` | Connected world indicator |
| Center | `(Activity: N)` | Worlds with unseen output |
| Right | `HH:MM` | Current time |

## Sending Commands

Simply type your command and press `Enter`. For example:

```
look
north
say Hello, world!
```

Commands are sent to the currently selected world.

## Essential Keyboard Shortcuts

| Keys | Action |
|------|--------|
| `Up/Down` | Switch between active worlds |
| `PageUp/PageDown` | Scroll output history |
| `Tab` | Release one screenful when paused |
| `Ctrl+P/N` | Navigate command history |
| `Ctrl+U` | Clear input line |
| `F1` | Open help popup |
| `/quit` | Exit Clay |

## Quick Command Reference

| Command | Description |
|---------|-------------|
| `/help` | Show help popup |
| `/worlds` | Open world selector |
| `/worlds -e` | Edit current world |
| `/connections` or `/l` | List connected worlds |
| `/disconnect` or `/dc` | Disconnect current world |
| `/setup` | Open global settings |
| `/actions` | Open actions/triggers editor |
| `/quit` | Exit Clay |

## Next Steps

- Read the **Interface Overview** chapter to understand all screen elements
- Check **Commands** for the full command reference
- Set up **Actions** to automate responses to MUD output
- Configure **Settings** for spell checking, themes, and more

\newpage

