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

### /chat (or /discord, /slack)

Commands for Slack and Discord worlds: `/chat` shows status, `/chat channels` lists
channels, `/chat to #channel` (or `@user`) changes where typing goes, and
`/chat msg <target> <text>` sends one message. See the
[Chat Worlds](#chat-worlds-discord-and-slack) chapter.

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
- Input height - The console's input area height (1-15); the GUI, the web page and the Android app each keep their own, set from their own Settings
- Console Theme - Dark or Light
- GUI Theme - Dark, Light, Dark - New or Light - New (see Themes in the Interface chapter)
- TLS Proxy - Enable TLS connection preservation
- ANSI Music - Enable music playback

### /web
Open Web Settings popup for HTTP/WebSocket configuration.

```
/web
```

See the **Web Interface** chapter for details.

### /reach
Show how other devices can reach this Clay: LAN/VPN/public addresses, the
Windows Firewall verdict, the router (UPnP) mapping state, and exactly what to
type on the other device. The same panel is the **Remote Access** button in `/web`.

```
/reach              Show the report
/reach --refresh    Re-check the firewall and router mapping (-r)
/reach --lookup     Look up the public IP via checkip.amazonaws.com; never automatic (-l)
```

See the **Web Interface** chapter, "Reaching Clay from outside your network".

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
/reload              (stays in the console or the GUI it is in)
/reload --console    (comes back as the console)
/reload --gui        (comes back as the GUI)
```

`--console` and `--gui` switch the interface at the reload, when the other one can run
there: the GUI needs a display (and a build with a GUI; on Windows and macOS, not a session
reached over SSH), the console a terminal in the foreground. Otherwise Clay says why and doesn't reload. Typed in a GUI or console attached
to another Clay, they restart that program as the other kind, still attached to the same
server; in a browser or the phone app, which have neither, they are answered with a line.

Also triggered by:
- `Ctrl+R` keyboard shortcut
- `SIGUSR1` signal (`kill -USR1 $(pgrep clay)`)

See the **Hot Reload** chapter for details.

### /update
Update Clay to the latest GitHub release.

```
/update [-f]
```

Typed in the console (or in a GUI that runs Clay itself), it checks GitHub, downloads
this platform's build, installs it over the running binary and hot-reloads, keeping your
connections. `-f` installs the latest release even when it is not newer. An update that
works says two lines:

```
Updating Clay to v1.7.10 from v1.7.9
Reload successful
```

Already up to date, or a failed download, is one line saying so.

`clay --upgrade` does the same download and install from the command line, without
starting the console or a GUI, and exits; it ends with a line saying where the new version
is installed (there is no reload to say so).

Typed in a remote client - the desktop GUI attached with `--gui=host`, the SSH console,
the web page, the Android app - it updates what can be updated: first the Clay you are
attached to (it hot-reloads; the client reconnects), then the client's own binary, which
restarts attached to the same server. The client's lines say which program they are about:

```
Server: updating Clay to v1.7.10 from v1.7.9
Reload successful
This GUI: updating Clay to v1.7.10 from v1.7.9
```

The two steps never run at the same time. When the client and the server are the same
binary on the same machine, the client notices that the server's install already
replaced its file and simply restarts. A browser page or the Android app has no binary of
its own to update; a multiuser server is not updated by its users.

### /version
Show which build of Clay is running.

```
/version
```

In the console, or a GUI or Android app that runs Clay itself, it is one line:

```
Clay v1.7.11 (build 26/10/17 23:58-1d4a8b0a) [linux/x86_64]
```

In a client attached to another Clay - the desktop GUI with `--gui=host` or
`--web-gui=host`, the SSH or `--console=host` console, the Android app connected to a
server - it names both builds, this program's first:

```
Clay: Local  v1.7.12 (build 26/10/19 16:24-4049c2a4) [windows/x86_64]
      Remote v1.7.11 (build 26/10/17 23:58-1d4a8b0a) [linux/x86_64]
```

While the client's connection is down it shows its own line alone. A web page is the
server's own, so a browser shows the server's one line. `^X^V` shows the same.

### /say
Speak text aloud.

```
/say <text>
```

Reads `<text>` aloud where it was typed, with that interface's own text-to-speech (TTS:
Local, Kokoro or Edge - see the Settings chapter); a browser page or the Android app
reads it with the device's own voice. With TTS off the console says nothing but still
shows the line.

### /tts
Text-to-speech status, and the Kokoro voice's files.

```
/tts                  status: mode, voice, what is installed
/tts install          download Kokoro's runtime and model
/tts remove           delete them again
/tts voices           list the 54 Kokoro voices
/tts voice <name>     pick one (af_heart, am_michael, bf_emma, ...)
/tts test             speak a test line in the current mode
```

Kokoro is an offline neural voice (Kokoro-82M, Apache-2.0). It is not built into Clay:
`/tts install` fetches its runtime (sherpa-onnx, for this machine's platform) and the
model from the sherpa-onnx GitHub releases into `~/.clay/tts` (`~/clay/tts` on Windows;
`CLAY_TTS_DIR` overrides). The model comes in two builds and the install picks the one
this CPU runs fastest: the small int8 build (about 160 MB to download, 230 MB on disk)
where the CPU has 8-bit matrix instructions - x86 with AVX-512 VNNI, and ARM - and the
full-precision build (about 340 MB, 400 MB on disk) on an x86 CPU with only AVX2, where
int8 runs up to four times slower and can fall behind real time. `/tts status` names the
build installed, and `/tts install` on a machine that has the other switches it. The
install runs in the background and reports as it goes; afterwards set TTS to Kokoro in
`/setup` (or the GUI's Settings). `/tts status` and `/tts voice` are about the interface
you type them in: each keeps its own text-to-speech. Until
the files are there, Kokoro mode speaks with the Local engine, so nothing goes silent -
and Clay says so: once when Kokoro is selected without them, and once more the first
time a line is spoken that way, naming the command to run.

The model (about a second to load) is loaded in the background as soon as Kokoro is
selected, or at startup when it already is, so the first line spoken is as quick as any
other; each sentence plays as soon as it is synthesized. Kokoro is the most natural of
the three engines and also the heaviest: it uses every CPU the machine has (up to eight)
for the moment it takes to make a sentence, and on a desktop CPU that is a few times
faster than speaking, which is enough for a MUD, but not on a Raspberry Pi. With debug
mode on, `debug.log` records how long each line took to start. A new line always cuts
off the one being spoken, as with the Local engine. Between lines Kokoro costs nothing:
its audio stream is opened by the first line and closed again after half a minute of
silence (an open stream keeps the sound server mixing), and switching TTS to another
engine, or `/tts remove`, unloads the model and its threads until Kokoro is chosen again.

Where it works: the GUI builds on Linux, Windows and macOS, and the console when run
from one of those binaries (`clay` with no flags). The static Linux console build
(`clay-linux-x86_64-musl`) cannot load the runtime and says so; the Android app and
Termux keep their own engines. On Linux the voice (and the MUD's sounds) play through
the `pipewire` or `pulse` ALSA device when one exists, so they sit under the desktop's
mixer and volume like any other program; `/tts status` names the device after the first
line spoken. The voice plays wherever the lines are spoken: on the Clay itself for its
console, and on your own machine for a desktop GUI attached to a Clay elsewhere. In that GUI, `/tts`, `/tts install`, `/tts remove` and `/tts test` are about
your machine's files, and its notice says "not installed on this machine"; `/tts voices`
and `/tts voice` are the shared setting and go to the server. (`clay --tts-install` on
that machine does the same install from a shell.)

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

