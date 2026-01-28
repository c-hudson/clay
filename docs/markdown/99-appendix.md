# Appendices

## A. Character Encodings

### UTF-8

Standard Unicode encoding (default):
- Full Unicode character support
- Variable-width encoding (1-4 bytes per character)
- Most compatible with modern systems

### Latin1 (ISO-8859-1)

Western European encoding:
- 256 characters (single byte)
- Direct byte-to-Unicode mapping
- Common for older MUDs

### Fansi (CP437)

IBM PC character set:
- Box drawing characters: ─│┌┐└┘├┤┬┴┼
- Block elements: ░▒▓█
- Line drawing symbols
- Common for BBS-style MUDs

## B. WebSocket Protocol

Clay's WebSocket server uses JSON messages for communication.

### Message Types (Client → Server)

```json
{"type": "AuthRequest", "password_hash": "<sha256>"}
{"type": "SendCommand", "world": "name", "command": "text"}
{"type": "SwitchWorld", "world": "name"}
{"type": "ConnectWorld", "world": "name"}
{"type": "DisconnectWorld", "world": "name"}
{"type": "MarkWorldSeen", "world": "name"}
{"type": "Ping"}
```

### Message Types (Server → Client)

```json
{"type": "AuthResponse", "success": true}
{"type": "InitialState", "worlds": [...], "settings": {...}}
{"type": "ServerData", "world": "name", "data": "text", "timestamp": 123}
{"type": "WorldConnected", "world": "name"}
{"type": "WorldDisconnected", "world": "name"}
{"type": "WorldSwitched", "world": "name"}
{"type": "PromptUpdate", "world": "name", "prompt": "text"}
{"type": "UnseenCleared", "world": "name"}
{"type": "UnseenUpdate", "world": "name", "count": 5}
{"type": "ActivityUpdate", "count": 2}
{"type": "Pong"}
```

### Authentication Flow

1. Client connects to WebSocket
2. If IP is whitelisted, server sends InitialState immediately
3. Otherwise, client sends AuthRequest with SHA-256 hash of password
4. Server responds with AuthResponse
5. On success, server sends InitialState

### Password Hashing

```javascript
const hash = await crypto.subtle.digest('SHA-256',
    new TextEncoder().encode(password));
const hashHex = Array.from(new Uint8Array(hash))
    .map(b => b.toString(16).padStart(2, '0'))
    .join('');
```

## C. Configuration File Format

Settings are stored in `~/.clay.dat` using INI-like format.

### Global Section

```ini
[global]
more_mode_enabled=true
spell_check_enabled=true
temp_convert_enabled=false
world_switch_mode=unseen_first
show_tags=false
input_height=3
console_theme=dark
gui_theme=dark
tls_proxy_enabled=false
ansi_music_enabled=false
websocket_enabled=false
websocket_port=9002
websocket_password_hash=<sha256>
websocket_use_tls=false
websocket_cert_file=/path/to/cert.pem
websocket_key_file=/path/to/key.pem
websocket_allow_list=192.168.1.100,192.168.1.101
websocket_nonsecure_enabled=false
websocket_nonsecure_port=9003
http_enabled=false
http_port=9000
https_enabled=false
https_port=9001
```

### World Sections

```ini
[world:MyMUD]
hostname=mud.example.com
port=4000
user=username
password=secret
use_ssl=true
log_file=/home/user/logs/mymud.log
encoding=utf8
auto_connect_type=connect
keep_alive_type=nop
keep_alive_cmd=
```

### Value Types

| Type | Format | Example |
|------|--------|---------|
| Boolean | `true`/`false` | `more_mode_enabled=true` |
| Integer | Decimal number | `port=4000` |
| String | Plain text | `hostname=mud.example.com` |
| Enum | Lowercase identifier | `encoding=utf8` |

### Encoding Values

- `utf8` - UTF-8 (default)
- `latin1` - ISO-8859-1
- `fansi` - CP437

### Auto-Login Types

- `connect` - Send "connect user password"
- `prompt` - Send on telnet prompts
- `moo_prompt` - MOO-style prompts

### Keepalive Types

- `nop` - Telnet NOP
- `custom` - Custom command
- `generic` - Generic help command

### World Switch Modes

- `unseen_first` - Priority to worlds with unseen output
- `alphabetical` - Alphabetical order

## D. Command Reference

### Slash Commands (/)

| Command | Description |
|---------|-------------|
| `/help` | Open help popup |
| `/quit` | Exit Clay |
| `/worlds` | Open world selector |
| `/worlds <name>` | Connect to world |
| `/worlds -e [name]` | Edit world |
| `/worlds -l <name>` | Connect without auto-login |
| `/connections`, `/l` | List connections |
| `/disconnect`, `/dc` | Disconnect current world |
| `/send [opts] <text>` | Send text to world(s) |
| `/setup` | Open global settings |
| `/web` | Open web settings |
| `/actions` | Open actions editor |
| `/reload` | Hot reload |
| `/testmusic` | Test ANSI music |
| `/notify <msg>` | Send notification (Android) |
| `/gag` | In actions: hide matched line |

### TF Commands (#)

| Command | Description |
|---------|-------------|
| `#set var val` | Set variable |
| `#unset var` | Remove variable |
| `#let var val` | Local variable |
| `#echo msg` | Display message |
| `#send text` | Send to MUD |
| `#expr expr` | Evaluate expression |
| `#test expr` | Boolean test |
| `#if ... #endif` | Conditional |
| `#while ... #done` | While loop |
| `#for ... #done` | For loop |
| `#def name = body` | Define macro |
| `#undef name` | Remove macro |
| `#list [pat]` | List macros |
| `#bind key = cmd` | Key binding |
| `#load file` | Load script |
| `#save file` | Save macros |

## E. ANSI Color Codes

### Standard Colors (0-7)

| Code | Color |
|------|-------|
| 0 | Black |
| 1 | Red |
| 2 | Green |
| 3 | Yellow |
| 4 | Blue |
| 5 | Magenta |
| 6 | Cyan |
| 7 | White |

### Bright Colors (8-15)

Add 8 to standard color number.

### 256-Color Mode

```
ESC[38;5;<n>m    # Foreground
ESC[48;5;<n>m    # Background
```

Where n is:
- 0-7: Standard colors
- 8-15: Bright colors
- 16-231: 6x6x6 color cube
- 232-255: Grayscale

### True Color (24-bit)

```
ESC[38;2;<r>;<g>;<b>m    # Foreground
ESC[48;2;<r>;<g>;<b>m    # Background
```

## F. Telnet Protocol Reference

### IAC Commands

| Value | Name | Purpose |
|-------|------|---------|
| 255 | IAC | Interpret As Command |
| 254 | DONT | Refuse option |
| 253 | DO | Request option |
| 252 | WONT | Refuse to perform |
| 251 | WILL | Agree to perform |
| 250 | SB | Subnegotiation begin |
| 249 | GA | Go Ahead |
| 241 | NOP | No Operation |
| 240 | SE | Subnegotiation end |
| 239 | EOR | End of Record |

### Option Codes

| Value | Name | Description |
|-------|------|-------------|
| 3 | SGA | Suppress Go Ahead |
| 24 | TTYPE | Terminal Type |
| 25 | EOR | End of Record |
| 31 | NAWS | Window Size |

\newpage

