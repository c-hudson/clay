# Settings

Clay has three levels of settings: global settings, per-world settings, and web/server settings.

## Global Settings (/setup)

Open with `/setup` command:

![Global Settings](images/tui/setup.png)

### Available Options

| Setting | Description | Default |
|---------|-------------|---------|
| More mode | Enable more-style pausing | On |
| Spell check | Enable spell checking | On |
| Temp convert | Auto-convert temperatures in input | Off |
| World Switching | World cycling behavior | Unseen First |
| Show tags | Show MUD tags at line start | Off |
| Input height | Default input area height (1-15) | 3 |
| Console Theme | Terminal color scheme | Dark |
| GUI Theme | Remote GUI color scheme | Dark |
| TLS Proxy | Preserve TLS connections across reload | Off |
| ANSI Music | Enable ANSI music playback | Off |

### World Switching Modes

**Unseen First:**
- If any OTHER world has unseen output, switch to the world that received unseen output first (oldest)
- Otherwise, switch alphabetically

**Alphabetical:**
- Always switch to the alphabetically next world by name
- Wraps from last world back to first

### Temperature Conversion

When enabled (and F2/Show tags is on), temperatures typed in the input area are auto-converted:
- Type `32F ` → Shows `32F(0C) `
- Type `100C ` → Shows `100C(212F) `

Useful for international MUD players.

## Per-World Settings (/worlds -e)

Open with `/worlds -e` or edit from World Selector:

![World Editor](images/tui/world-editor.png)

### Connection Settings

| Setting | Description |
|---------|-------------|
| World name | Display name for this world |
| Hostname | Server address (e.g., mud.example.com) |
| Port | Server port (e.g., 4000) |
| Use SSL | Enable TLS/SSL connection |

### Authentication Settings

| Setting | Description |
|---------|-------------|
| User | Username for auto-login |
| Password | Password for auto-login (plaintext) |
| Auto login | Login method (see below) |

**Auto-login types:**

| Type | Behavior |
|------|----------|
| Connect | Sends `connect <user> <password>` after 500ms |
| Prompt | Sends username on first telnet prompt, password on second |
| MOO_prompt | Like Prompt, but sends username again on third prompt |

Auto-login only triggers if BOTH username AND password are configured.

### Keep-Alive Settings

| Setting | Description |
|---------|-------------|
| Keep alive | Keepalive type (NOP, Custom, Generic) |
| Keep alive cmd | Custom command (when type is Custom) |

**Keepalive types:**

| Type | Behavior |
|------|----------|
| NOP | Sends telnet NOP command (IAC NOP) every 5 minutes |
| Custom | Sends your custom command |
| Generic | Sends `help commands ##_idler_message_<rand>_###` |

### Other Settings

| Setting | Description |
|---------|-------------|
| Log file | Path to output log file (append mode) |
| Encoding | Character encoding (UTF-8, Latin1, Fansi) |

**Encoding types:**

| Type | Description |
|------|-------------|
| UTF-8 | Standard UTF-8 (default) |
| Latin1 | ISO-8859-1 for older MUDs |
| Fansi | CP437-like with box drawing characters |

## Web Settings (/web)

Open with `/web` command:

![Web Settings](images/tui/web.png)

### WebSocket Server (Secure)

| Setting | Description | Default |
|---------|-------------|---------|
| WS enabled | Enable secure WebSocket server | Off |
| WS port | Port for wss:// connections | 9002 |
| WS password | Authentication password | (required) |
| WS Allow List | CSV of IPs that can be whitelisted | (empty) |
| TLS Cert File | Path to TLS certificate | (required for TLS) |
| TLS Key File | Path to TLS private key | (required for TLS) |
| WS Use TLS | Enable TLS for WebSocket | Off |

### WebSocket Server (Non-Secure)

| Setting | Description | Default |
|---------|-------------|---------|
| WS Nonsecure | Enable non-secure WebSocket | Off |
| WS NS port | Port for ws:// connections | 9003 |

### HTTP/HTTPS Web Interface

| Setting | Description | Default |
|---------|-------------|---------|
| HTTP enabled | Enable HTTP web server | Off |
| HTTP port | Port for HTTP | 9000 |
| HTTPS enabled | Enable HTTPS web server | Off |
| HTTPS port | Port for HTTPS | 9001 |

**Note:** HTTP automatically starts the non-secure WebSocket server if needed.

## Settings Persistence

Settings are automatically saved to `~/.clay.dat`:

- Global settings saved when closing the Settings popup
- World settings saved when closing the World Editor
- File format is INI-like with `[global]` and `[world:name]` sections

### Example ~/.clay.dat

```ini
[global]
more_mode_enabled=true
spell_check_enabled=true
world_switch_mode=unseen_first
websocket_enabled=false
websocket_port=9002

[world:MyMUD]
hostname=mud.example.com
port=4000
user=myname
password=secret
use_ssl=true
encoding=utf8
auto_connect_type=connect
```

\newpage

