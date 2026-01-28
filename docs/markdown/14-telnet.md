# Telnet Features

Clay includes comprehensive telnet protocol support for proper MUD server communication.

## Automatic Negotiation

Clay automatically handles telnet negotiation:
- Detects telnet mode when IAC sequences are received
- Responds appropriately to server requests
- Strips telnet sequences from displayed output

## Supported Telnet Options

| Option | Code | Description |
|--------|------|-------------|
| SGA | 3 | Suppress Go Ahead |
| TTYPE | 24 | Terminal Type |
| EOR | 25 | End of Record |
| NAWS | 31 | Negotiate About Window Size |

### SGA (Suppress Go Ahead)

Accepts server's WILL SGA with DO SGA. Most modern MUDs use SGA for character-at-a-time mode.

### TTYPE (Terminal Type)

Reports terminal type to the server:
- Uses TERM environment variable (e.g., "xterm-256color")
- Falls back to "ANSI" if TERM is not set
- Responds to SB TTYPE SEND with SB TTYPE IS \<terminal\>

### EOR (End of Record)

Alternative prompt marker, treated the same as GA:
- When received, text from last newline is identified as prompt
- Prompt is displayed at the start of input area

### NAWS (Negotiate About Window Size)

Reports window dimensions to the server:
- Sends smallest width × height across all connected clients
- Updates sent when terminal resizes
- Updates sent when web/GUI client dimensions change
- Dimensions tracked per-world, reset on disconnect

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

\newpage

