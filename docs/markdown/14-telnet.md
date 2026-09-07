# Telnet Features

Clay includes comprehensive telnet protocol support for proper MUD server communication.

## Automatic Negotiation

Clay automatically handles telnet negotiation:
- Detects telnet mode when IAC sequences are received
- Sends its own opening offer at connect time (see **Opening Negotiation** below), not just
  answers to what the server asks
- Responds appropriately to server requests
- Strips telnet sequences from displayed output

## Opening Negotiation

Older Clay versions only ever *answered* negotiation the server started — a server that
never speaks first (the norm on plain MUSH/MOO servers) got no GMCP, MSDP, MSSP, MCCP2, or
CHARSET at all, because nothing on Clay's side would offer first.

Clay now sends an opening offer immediately on connect, controlled by the per-world
**Negotiate** setting (World Editor; defaults on). When Negotiate is on, Clay sends, in
order:

`WILL TTYPE`, `WILL NAWS`, `DO CHARSET`, `DO GMCP`, `DO MSDP`, `DO MCCP2`, `DO MSSP`, and —
only if the world's **MSP Sound** setting is also on — `DO MSP`.

SGA and EOR are never offered this way: Clay behaves identically whether either is on or
not, so there's nothing to gain by asking first.

Turn Negotiate off if a server reacts badly to being spoken to before it speaks first. With
it off, Clay falls back to answering only what the server itself offers — the old behavior.

## Supported Telnet Options

| Option | Code | Direction |
|--------|------|-----------|
| ECHO | 1 | Server-driven only. Clay answers the server's `WILL`/`WONT ECHO` unconditionally, but never asks for it itself. Drives password masking — see below. |
| SGA | 3 | Server → Clay. Clay accepts `WILL SGA` from the server; it never asks first. |
| TTYPE | 24 | Clay → Server. Clay offers `WILL TTYPE` (at connect if Negotiate is on, or whenever the server asks with `DO TTYPE`) and answers `SB TTYPE SEND` with a 3-answer cycle — see below. |
| EOR | 25 | Both directions. Clay accepts `WILL EOR` from the server (the server will send EOR) and answers a `DO EOR` request (Clay will send EOR) — but never asks for it first, same as SGA. |
| NAWS | 31 | Clay → Server. Clay offers `WILL NAWS` (at connect if Negotiate is on, or whenever asked) and reports window size. |
| CHARSET | 42 | Both directions. Clay asks `DO CHARSET` at connect (if Negotiate is on) and also accepts `WILL CHARSET` if the server offers first. |
| MSDP | 69 | Both directions, same pattern as CHARSET. |
| MSSP | 70 | Both directions, same pattern as CHARSET. |
| MCCP2 | 86 | Both directions, same pattern as CHARSET. Also see **Compression and Hot Reload** below. |
| MSP | 90 | Both directions, same pattern as CHARSET — but Clay only asks first (`DO MSP`) when the world's **MSP Sound** setting is on. See **MSP (Sound Effects)** below for what this option actually covers. |
| GMCP | 201 | Both directions, same pattern as CHARSET. |
| MCP 2.1 | in-band, no option number | Not a telnet option at all — detected from ordinary text lines that begin with `#$#`. See **MCP (MUD Client Protocol)** below. |

"Both directions" above means: if the server offers the option first (`WILL x`), Clay
accepts it (`DO x`); and, independently, if Negotiate is on, Clay also asks for it first
(`DO x`) at connect time, which the server can then accept (`WILL x`). Either order reaches
the same negotiated state.

### ECHO and Password Masking

`IAC WILL ECHO` — the server telling Clay it will handle echoing, the standard way a MUD
asks a client to stop echoing locally so a typed password isn't shown in the clear — now
actually masks input, not just the wire reply (`DO ECHO`):

- The input area shows `*` for every typed character instead of the real text
- Typed text is excluded from command history (no arrow-key recall)
- Typed text is excluded from the per-world session log
- Typed text is excluded from scrollback (not archived, not `/recall`-able)

Masking clears when the server sends `IAC WONT ECHO` (Clay answers `DONT ECHO`), or when
the world disconnects/reconnects. Web, GUI, and Android clients mask their own input the
same way, mirrored from the server's echo state.

This is separate from the WONT-ECHO-based prompt-boundary heuristic used for MOO-style
auto-login (see **Auto-Login Prompt Detection** below) — one decides which text is safe to
keep, the other decides which text becomes the displayed prompt.

### SGA (Suppress Go Ahead)

Accepts server's WILL SGA with DO SGA. Most modern MUDs use SGA for character-at-a-time
mode. Clay does not ask for it first.

### TTYPE (Terminal Type)

Clay offers to report its terminal type (`WILL TTYPE`), either at connect (if Negotiate is
on) or whenever the server asks with `DO TTYPE`.

When the server sends `SB TTYPE SEND`, Clay answers with a three-answer cycle that repeats
its last answer forever — the standard MTTS convention servers use to know when the
client's list has ended:

1. First request: the client name, `CLAY`
2. Second request: the terminal type — the `TERM` environment variable, uppercased, or
   `ANSI` if `TERM` is unset
3. Third and every later request: `MTTS <bitmask>`

The MTTS bitmask always reports ANSI, UTF-8, 256-color, and truecolor support (all
genuinely implemented in Clay's ANSI parser and renderer); it adds the SSL bit when the
connection is using TLS. Clay does not claim VT100 line-drawing, mouse tracking,
screen-reader, proxy, MNES, or MSLP support — none of those exist in Clay today.

### EOR (End of Record)

Alternative prompt marker, treated the same as GA:
- When received, text from last newline is identified as prompt
- Prompt is displayed at the start of input area

Clay accepts EOR in either direction (the server offering to send it, or the server asking
Clay to send it) but never asks for it first.

### NAWS (Negotiate About Window Size)

Reports window dimensions to the server:
- Sends smallest width × height across all connected clients
- Updates sent when terminal resizes
- Updates sent when web/GUI client dimensions change
- Dimensions tracked per-world, reset on disconnect

### CHARSET (RFC 2066)

Lets the server and Clay agree on a character encoding mid-connection. If the world's
Encoding setting is the default (UTF-8), Clay accepts whatever the server offers that it
understands. If the user explicitly chose Latin-1 or Fansi for this world, a same-session
CHARSET offer cannot silently override that choice: Clay accepts the offer only if it
includes the chosen encoding, and rejects it outright otherwise, rather than accepting
something the user didn't ask for and never actually switching to it.

### MSSP (MUD Server Status Protocol)

Server-volunteered status data (name, players online, uptime, genre, and so on) as
name/value pairs. Read-only — Clay has nothing to send back beyond accepting the option.
View the current world's MSSP data with `/mssp` (see the **Commands** chapter).

### MSDP (MUD Server Data Protocol)

Structured name/value data exchange with the server, including nested tables and arrays.
Clay both receives MSDP data pushed by the server and can send outbound MSDP commands with
`/msdp <LIST|REPORT|UNREPORT|SEND> [target]` (see the **Commands** chapter).

Every MSDP variable Clay receives also feeds the per-world status display — the console's
status line, the web/GUI/Android status panel, and `/stats` (see the **Interface Overview**,
**Web Interface**, and **Commands** chapters), except MSDP's own protocol bookkeeping
(`REPORTABLE_VARIABLES` and its siblings), which never becomes a stat.

Clay asks for this data itself rather than waiting on the server or on you: as soon as MSDP
negotiates, Clay sends `LIST REPORTABLE_VARIABLES` to ask what the server can report, then
sends a `REPORT` for everything named in the reply (capped at 100 variables, so a server
advertising an enormous list can't turn one reply into an unbounded burst of requests). This
is why MSDP data now shows up without needing to type `/msdp REPORT` by hand — that command
still exists for reporting something the automatic pass didn't catch, or for a server that
doesn't answer `LIST` at all.

### MCCP2 (MUD Client Compression Protocol v2)

Once the server activates compression (`IAC SB MCCP2 IAC SE`), all further data on the
connection is a zlib stream that Clay decompresses transparently.

**Compression and Hot Reload:** the zlib decompressor's sliding window lives only in the
running process's memory and cannot survive a hot reload's `exec`. A world with active
MCCP2 compression is disconnected during `/reload` (or crash recovery) with an explanatory
message — `Compressed (MCCP2) connection was closed during reload. Use /worlds to
reconnect.` — instead of being handed to a fresh process that would otherwise try to parse
compressed bytes as plaintext and show garbage. Use `/worlds` to reconnect afterward.

### MSP (Sound Effects)

MSP (MUD Sound Protocol) triggers one-shot sound and music playback from `!!SOUND(...)`/
`!!MUSIC(...)` markers embedded in the server's text output. It is **not** a general music
system: there is no persistent background-score tracking, and MSP music is not restarted
when you switch away from and back to a world (unlike GMCP `Client.Media` audio, which
Clay does track and resume on world switch). A background world's `!!MUSIC(Off)`/
`!!SOUND(Off)` still silences anything already playing, but a fresh trigger on a world
you're not currently viewing is simply dropped, not queued for later.

Controlled by the per-world **MSP Sound** setting (World Editor; defaults on). Turning it
off means MSP does not exist as far as this world's connection is concerned: Clay never
asks the server for the option, and any `!!SOUND(...)`/`!!MUSIC(...)` text that arrives
anyway is left in the output exactly as sent (not recognized, not stripped, not played) —
this is deliberately different from "recognized but silenced."

### GMCP (Generic MUD Communication Protocol)

Structured JSON message exchange with the server, keyed by dotted package names (e.g.
`Char.Items`, `Room.Info`). Configure which packages to request per world with the GMCP
field in the World Editor. GMCP also carries server-driven sound/music via
`Client.Media.*`, which — unlike MSP — Clay does resume when you switch back to a world.

`Char.*` packages (e.g. `Char.Vitals`) feed the same per-world status display as MSDP does —
`/stats`, the console status line, and the web/GUI/Android status panel (see the **Commands**,
**Interface Overview**, and **Web Interface** chapters). The GMCP field defaults to
`Client.Media 1, Char 1`, so a new world declares support for the whole `Char` package family
as well as media — a spec-following server has no reason to withhold `Char.*` data from a
client that asked for it this way. An existing world created before this default changed picks
it up automatically too, unless its GMCP field (World Editor) was already customised: the
upgrade only ever replaces a value that is *exactly* the old `Client.Media 1` default, so
nobody's own edit is silently overwritten. Whether `Char.*` data
actually arrives still ultimately depends on the MUD — some send it unprompted to any
GMCP-enabled client regardless of what was requested, others honor only specific sub-packages
rather than the whole family, and a MOO/MUSH implementing no GMCP at all sends none either way.
GMCP itself is a Diku-family convention — MOOs and MUSHes generally don't
implement it at all, so the status display correctly never appears on those.

### MCP (MUD Client Protocol)

MCP 2.1 is not a telnet option — it is detected from ordinary output lines that begin with
`#$#`, mainly seen on MOO-family servers. Controlled by the per-world **MCP Edit** setting
(World Editor; defaults on); when on, Clay implements the core MCP 2.1 handshake/framing
and the `dns-org-mud-moo-simpleedit` package, which makes `@edit` open a real text editor
instead of dumping verb source into the scrollback. Turning it off makes Clay ignore
`#$#`-prefixed lines completely — they display as plain text instead of being parsed.

Every MCP message after the initial handshake must carry Clay's own per-connection
authentication key; a message with a wrong or missing key is treated as unauthenticated and
hidden rather than acted on, so another player's chat text that happens to contain `#$#`
can't forge protocol messages.

**MCP does not survive a hot reload.** Unlike MCCP2, a `/reload` does not disconnect an
MCP-enabled world — the socket itself survives the `exec` fine — but the MCP handshake
state (the per-connection key, any in-progress multiline edit) lives only in the old
process's memory and is gone afterward. MCP silently stops working for that world until it
reconnects; there is no automatic re-handshake, because the server has no reason to resend
its one-time invite mid-session. This is a known, deliberate scope limit, not a bug.

## Not Supported

Clay does not implement MXP (MUD eXtension Protocol) or MSLP (MUD Simple Link Protocol).
Both were evaluated and deliberately not built — MSLP's "click a link to send a command"
model would require new click-to-send interaction machinery Clay's console and web/GUI
clients don't have today, and reinterpreting plain underline styling as a clickable command
by convention alone risks misfiring on any MUD that underlines ordinary text for emphasis.

## Prompt Detection

Clay detects prompts using telnet GA (Go Ahead) or EOR (End of Record):

### How It Works

1. Server sends output ending with IAC GA or IAC EOR
2. Text after the last newline is identified as the prompt
3. Prompt is stored per-world
4. Prompt is displayed at the start of the input area (cyan)
5. Prompt is NOT shown in output area

### Prompt Handling

- Trailing spaces are normalized: stripped, then one space added
- ANSI codes in prompts are preserved
- Cursor positioning uses visible prompt length
- Prompt cleared when user sends a command

### Auto-Login Prompt Detection

When Auto Login is set to "Prompt" or "MOO_prompt":
- First telnet prompt: Username sent automatically
- Second telnet prompt: Password sent automatically
- MOO_prompt third prompt: Username sent again

Prompts that are auto-answered are cleared and not displayed.

## Keepalive

Configurable keepalive prevents idle disconnection:

### When Sent

- Every 5 minutes of inactivity (no data sent)
- Only when in telnet mode
- Per-world timing

### Keepalive Types

Configure in World Settings:

| Type | Behavior |
|------|----------|
| NOP | Sends telnet NOP command (IAC NOP) |
| Custom | Sends your custom command |
| Generic | Sends `help commands ##_idler_message_<rand>_###` |

### Custom Keepalive

Set "Keep alive" to "Custom" and configure "Keep alive cmd":

```
# Example custom keepalive
look
```

The generic option sends a command that:
- Works on most MUDs (help commands)
- Includes random text to avoid pattern detection
- Produces minimal server output

## Timing Fields

Track connection activity:

| Field | Description |
|-------|-------------|
| last_send_time | When user last sent a command |
| last_receive_time | When data was last received |

These are:
- Initialized on connect
- Reset after /reload
- Used to calculate keepalive timing
- Shown in `/connections` output

## Line Buffering

Clay properly buffers incoming data:

### Safe Splitting

- `find_safe_split_point()` checks for incomplete sequences
- ANSI CSI sequences not split mid-sequence
- Telnet commands not split mid-command
- Remaining buffer flushed on connection close

### Partial Lines

Lines without trailing newlines (e.g., prompts) are handled specially:
- Displayed immediately
- `partial_line` tracks incomplete lines
- When more data arrives, the line is updated in-place
- Prevents duplicate lines from TCP read splitting

## Telnet Sequences

### Common Sequences

| Sequence | Name | Purpose |
|----------|------|---------|
| IAC GA | Go Ahead | End of output, prompt follows |
| IAC EOR | End of Record | Alternative prompt marker |
| IAC NOP | No Operation | Keepalive |
| IAC WILL x | Will | Server offers option x |
| IAC WONT x | Won't | Server refuses option x |
| IAC DO x | Do | Client should enable option x |
| IAC DONT x | Don't | Client should disable option x |
| IAC SB ... IAC SE | Subnegotiation | Option-specific data |

### IAC Values

- IAC: 255 (0xFF)
- WILL: 251
- WONT: 252
- DO: 253
- DONT: 254
- SB: 250
- SE: 240
- GA: 249
- EOR: 239
- NOP: 241

## Troubleshooting

### No Prompt Detected

1. Verify MUD server sends GA or EOR
2. Check telnet mode is active (server sends IAC sequences)
3. Some MUDs require explicit telnet negotiation

### Wrong Terminal Type

1. Check TERM environment variable
2. Set explicitly: `TERM=xterm-256color ./clay`

### Disconnected for Idling

1. Enable keepalive in World Settings
2. Try different keepalive type
3. Reduce the 5-minute interval may require code change

### Garbled Output

1. Check character encoding in World Settings
2. Try Latin1 for older MUDs
3. Try Fansi for BBS-style output

### A Server Reacts Badly to Clay's Opening Offer

Some servers are confused by an unsolicited opening offer. Turn off the per-world
**Negotiate** setting so Clay only answers what the server asks first.

\newpage

