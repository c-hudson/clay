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
| Input height | Input area height in lines (1-15), kept per interface: the console's here, and one each for phone, tablet and desktop clients (the GUIs and a desktop browser are desktops), each set from that interface's own Settings, `Alt-Up/Down` or, in the GUI and web client, a drag of the input's top edge. In `settings.dat`: `input_height` (console), `input_height_phone`, `input_height_tablet`, `input_height_desktop`; a file from before the split starts all three at the console's | 3 |
| Clear Input on Send | TinyFugue's `%cleardone`. Off: each line you send stays in the input area above the next one until it scrolls off the top (see **Input Area**); `Ctrl+L` clears them. On: Enter clears the input area. Phones always clear. In `settings.dat`: `clear_input_on_send` | Off |
| Text Reveal | A newly arrived line is typed onto the screen left to right behind a glitching cursor, the way tfr's terminal reveal draws it. Kept per interface: the console's here, and one each for the native GUI, the WebView GUI, a browser and the Android app (per OS, as fonts are) and for a console attached to another Clay, each set from that interface's own Settings and Off until it is: Slow 30, Normal 60, Fast 120 or Super Fast 240 characters a second (200 in the native GUI and the Android app), or Custom, which asks for the number (1 to 10000) in a popup and then shows it as "Custom (n)", or New Only, which asks for a number the same way ("New Only (n)") but types on only text this interface hasn't shown yet (each line a little faster or slower than the last; a wrapped line's rows are typed on together, so it takes as long as its longest row; a long line is sped up so none takes over 10 s). Stepping through the choices with the keys never asks, nor does moving on to the next setting, so you can go past Custom to New Only: press Enter on Custom or New Only to type its number (or to change it), and Save asks for it if you haven't. Picture rows (FANSI) reveal cell by cell like text. The screen you switch to is typed on the same way, as if its text had just arrived; with New Only, only the lines that arrived since you last looked at that world are, and the rest is drawn whole at once, so new text stands out and a switch draws faster. A page held by more-mode is typed on when Tab or Page Down lets it through. Only the drawing changes: wrapping, scrolling, paging and more-mode are what they were, and a line restored by `/reload` or read from the archive is shown whole. In `settings.dat`: `text_reveal` (the console), `text_reveal.<interface key>` (the others); a custom speed is `custom:<n>`, a New Only one `new:<n>` | Off |
| Console Theme | Terminal color scheme | Dark |
| GUI Theme | Colour scheme of the GUI, the web page and the Android app: Dark, Light, Dark - New, Light - New or a custom theme from `theme.dat` | Dark |
| TLS Proxy | Preserve TLS connections across reload | Off |
| ANSI Music | Enable ANSI music playback | Off |
| New Indicator | How lines that arrived while you were in another world are marked once you switch to it. Character: a green ▶ in front of each one. Background: a green glow behind its first four characters, fading to the theme's background (black or white on the console). Custom: the same glow in another colour, chosen from a window of ten (Teal, Cyan, Blue, Indigo, Purple, Magenta, Red, Orange, Gold, Slate), each shown as the word "Clay" with its fade; it opens when you pick the entry (in the console, press Enter on it), and the entry then reads "Custom (Teal)". None: not marked. The marks go when you leave the world or press Ctrl+L, in every style, and the glows add no text, so lines wrap as without them. One global setting, shown in every interface's Settings. In `settings.dat`: `new_indicator` (`none`, `character`, `background`, `custom:<colour>`) | None |
| TTS | Text-to-speech for incoming lines, for this interface only: each interface keeps its own and sets it in its own Settings (see the table below). Off; Local (espeak or espeak-ng on Linux, `say` on macOS, the Windows voices; in a browser or the Android app, the device's own voice); Kokoro (an offline neural voice, installed once with `/tts install`); Edge (Microsoft's cloud voices, needs internet). A browser and the Android app offer Off and Local only, and the static Linux console offers no Kokoro. Each interface hears the lines of the world it shows, even in the background. F9 mutes the console's. | Off |
| Speak Mode | All: every line. Limit: only lines like `Name says, "..."` and later lines from the same speakers | All |
| Kokoro Voice | Which of Kokoro's 54 voices speaks, e.g. `af_heart`, `am_michael`, `bf_emma`; `/tts voices` lists them by language. Shown only where Kokoro can speak: not in a browser, the Android app or the static Linux console. In `settings.dat`: `tts_voice` (the console's), `tts.<interface>.voice` (the others') | af_heart |
| Picture Host | Where Import Picture uploads a picture (see **Importing a Picture**): catbox.moe, sxcu.net, x0.at or tmpfiles.org (2d: deleted after two days), each also "+ URL Short" (the link goes through `/url`'s shortening), or Off. In `settings.dat`: `picture_host` (`catbox`, `catbox+short`, ..., `off`) | catbox.moe |
| Separator Style | The look of the bar above the input. Clay: the gaps are spaces on the status bar colour, the look of the GUIs and the web page. TinyFugue: the gaps are underscores on the terminal background, as TinyFugue draws its status line. The look applies to TF's status line too once a script has changed it (see **Switching from TinyFugue**), in every interface; a `%status_pad` or `%status_attr` the script set is honoured in either look. Changing the setting from TinyFugue back to Clay hands the bar back to Clay: TF's layout and every `status_*` variable a script set go back to their defaults (set them again if wanted). One global setting, shown in every interface's Settings (the Theme Editor's Separator Bar group sets the same value). In `settings.dat`: `separator_style` (`web` for Clay, `tinyfugue`) | Clay |

### Settings Shown per Interface

Each interface's Settings shows only what acts in that interface. A setting that isn't
shown keeps its value; saving from there never changes it.

| Setting | Shown in |
|---------|----------|
| GUI Theme, Tabs, Icon Bar | the GUIs, the web page and the Android app; not the console's `/setup` |
| Keyboard Visible | the web page on a phone or tablet layout (the Android app, a mobile browser) |
| ZWJ Sequence | the console's `/setup` only |
| Clear Input on Send | everywhere but a phone, which always clears |
| Font sizes: Phone and Tablet | a browser and the Android app; the GUIs are desktops and show only Desktop |
| Separator Style | everywhere: one global setting, the look of Clay's bar in the console and of TF's status line in every interface |
| TTS, Speak Mode | everywhere, but each interface's own: the console's `/setup` sets the console's, a phone's Settings the phone's (the Android app's), a GUI's that GUI's, kept per program and OS like fonts |
| Kokoro Voice | where Kokoro can speak: the GUIs and the console (not its static Linux build); not a browser or the Android app, which speak with the device's own voice |

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
| World type | `MUD`, `MUD - Timed Prompt`, `Slack`, or `Discord`. The two MUD types connect identically; **MUD - Timed Prompt** additionally infers a prompt from a line the server leaves unfinished (see Prompt wait below) |
| Hostname | Server address (e.g., mud.example.com) |
| Port | Server port (e.g., 4000) |
| Use SSL | Enable TLS/SSL connection |

Slack and Discord worlds have their own fields instead (Token, Server/Workspace,
Send To, Show Only, and a **Fetch** button that fills in pick lists); see the
[Chat Worlds](#chat-worlds-discord-and-slack) chapter.

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

### Prompt Detection

Clay normally recognises a prompt only when the server marks it with telnet GA/EOR (or a
WONT-ECHO password prompt). Some MUDs never mark their prompts at all — Aardwolf's login
prompt is the reference case — so `Prompt`/`MOO_prompt` auto-login could never fire there.

| Setting | Description | Default |
|---------|-------------|---------|
| Prompt wait (ms) | Shown only for **MUD - Timed Prompt** worlds. A line the server leaves unfinished is treated as the prompt once nothing further has arrived for this long. Raise it on a slow link if a line that was merely arriving slowly gets taken for a prompt; lower it if prompts feel late | 1000 |

Plain `MUD` worlds never infer a prompt from timing, so a server that sends long lines in
several pieces cannot produce a false prompt there. If a world used to rely on the
unmarked-prompt detection introduced in v1.6.5, set its type to **MUD - Timed Prompt**.

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

### Protocol Settings

See the **Telnet Features** chapter for what each of these actually negotiates.

| Setting | Description | Default |
|---------|-------------|---------|
| MSP Sound | Play `!!SOUND(...)`/`!!MUSIC(...)` triggers the MUD sends (MUD Sound Protocol). Off means these triggers are not recognized at all — the option is never requested, and any trigger text that still arrives is left in the output unstripped and unplayed | On |
| MCP Edit | Support MCP (MUD Client Protocol), mainly used by MOOs — makes `@edit` open a real text editor instead of dumping verb source into the scrollback. Off means `#$#`-prefixed protocol lines display as plain text instead of being parsed | On |
| MCCP2 | Accept MCCP2 compression when the server offers it. Off declines it at negotiation; toggling it on a live connection starts or stops compression immediately. Compressed connections survive `/reload` | On |

## Web Settings (/web)

Open with `/web` command:

![Web Settings](images/tui/web.png)

### WebSocket/Web Server

There's a single server, not separate secure/non-secure WebSocket listeners — it
serves the web UI and WebSocket connections on one port, and is TLS-capable for
remote clients while always serving localhost plain (see `SECURITY-NOTES.md`). The
live settings keys (`persistence.rs`) are `http_port` (default 9000) and `web_path`
(the stealth URL prefix, default `clay`). Older config files may still contain a
handful of separate-port/enabled keys from a previous WebSocket layout —
`persistence.rs` loads and silently ignores all of them; they no longer do anything.

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
http_enabled=false
http_port=9000

[world:MyMUD]
world_type=mud
hostname=mud.example.com
port=4000
user=myname
password=secret
use_ssl=true
encoding=utf8
auto_connect_type=connect
```

\newpage

