# Features Reference

## Screen Layout

```
Output Area                    <- Takes up rest of screen, ANSI color support
Shows one world at a time
(no border)
 world [activity] [history]____time  <- Separator bar (underscores)
input line 1                          <- Input area (no border)
input line 2
input line 3
```

On startup, each world displays a colorful ASCII art splash screen with the tagline "A 90dies MUD client written today" and quick command hints.

### Separator Bar Components
- Status indicator (leftmost, 10 chars, black text on red background):
  - `More XXXX` - pending lines when paused (priority 1)
  - `Hist XXXX` - lines scrolled back in history (priority 2)
  - Underscores when neither active (dark gray)
  - Numbers are right-justified: 9999 → "9999", 10000 → " 10K", 999000 → "999K", 1000000+ → "Alot"
- Connection indicator and world name (only shown when connected):
  - Green ball (🟢) followed by world name (bold white)
  - When disconnected, this area is filled with underscores instead
- Activity indicator at position 24: `(Activity: X)` or `(Act X)` on narrow screens - count of worlds with unseen output (yellow, hidden if 0)
- Underscore padding fills remaining space (dark gray)
- Current time HH:MM format (right, cyan, no AM/PM)

## Character Encoding

Encoding is configurable per-world in the world settings popup.

- **Utf8**: Standard UTF-8 (default) - `String::from_utf8_lossy`
- **Latin1**: ISO-8859-1 - direct byte-to-Unicode mapping
- **Fansi**: CP437-like for MUD graphics - box drawing chars, block elements
- Raw bytes passed via `AppEvent::ServerData(world_idx, Vec<u8>)`, decoded in main loop
- Control characters filtered during decode (keeps tab, newline, escape for ANSI)

### Colored Square Emoji
- Colored square emoji (🟥🟧🟨🟩🟦🟪🟫⬛⬜) are rendered with proper colors
- Console: Replaced with ANSI true-color block characters (██) since terminal emoji fonts ignore foreground colors
- WebView GUI: Native browser rendering
- Web: Native emoji rendering
- Implementation: `colorize_square_emojis()` in encoding.rs, `has_colored_squares()` in GUI

### Display Width Handling
- Input area uses unicode display width for cursor positioning and line breaking
- Zero-width characters (U+200B, etc.) take no visual space
- Wide characters (CJK, emoji) take 2 columns
- Helper functions in `src/input.rs`: `display_width()`, `display_width_chars()`, `chars_for_display_width()`
- First line has reduced capacity due to prompt; subsequent lines use full terminal width

## Word Wrapping

All interfaces (console, web, GUI) use consistent word wrapping for long words:

- Words longer than 15 characters can break at: `] ) , \ / - _ & = ? ;`
- Period (`.`) excluded to avoid breaking filenames and numbers
- Opening brackets `[` `(` excluded to keep them with following content
- Underscore (`_`) included for identifiers
- Console: `wrap_ansi_line` tracks break opportunities
- Web: `insertWordBreaks()` inserts zero-width spaces after break characters
- GUI: `insert_word_breaks()` inserts zero-width spaces, skipping ANSI sequences

## MUD Tag Display (F2)

- MUD tags are prefixes matching two patterns at the start of lines:
  - `[name:] ` — colon before `]`, e.g., `[Public:] Hello`
  - `[name(content)optional] ` — non-empty parens, e.g., `[Chat(Bob)] Hello`
  - A space after `] ` is required
  - Empty parens not matched
  - Leading ANSI color codes before `[` are preserved
- When hidden (default), tags stripped from display but preserved in buffer
- When shown, full lines with timestamps displayed
- **Timestamps**: `HH:MM>` for today, `DD/MM HH:MM>` for previous days (cyan)
- **Gagged lines**: Lines hidden by `/gag` are also shown with F2
- **Temperature conversion**: When enabled in `/setup`, temperatures in input auto-converted (e.g., "32F " → "32F(0C) "). Only active when F2/show_tags mode is on.

## Actions

Actions are automated triggers that match incoming MUD output against patterns and execute commands.

### Action Processing
- Incoming lines checked against all action patterns as they arrive
- Pattern matching is case-insensitive
- ANSI color codes stripped before pattern matching
- World-specific actions only match for their configured world (empty = all worlds)
- Regexes are pre-compiled on the `Action` struct (`compiled_regex` field) and recompiled only when created, edited, toggled, or loaded

### Match Types
- **Regexp** (default): Pattern is a regular expression
- **Wildcard**: `*` matches any sequence, `?` matches single character. Use `\*` and `\?` for literals.

### Capture Group Substitution
- `$0` - Entire matched text
- `$1` through `$9` - Captured groups
- For **Regexp**: use parentheses for capture groups: `^(\w+) tells you: (.*)$`
- For **Wildcard**: each `*` and `?` becomes a capture group automatically
- Manual invocation: `/actionname args` — `$1-$9` are space-separated args, `$*` is all args

### Special Commands
- `/gag` in command list hides the matched line (stored for F2 viewing)
- Multiple commands separated by semicolons
- Commands starting with `/` processed as client commands; plain text sent to server

### Startup Actions
- Actions can have "Startup" enabled to run commands when Clay starts
- Fires on fresh start, hot reload, and crash recovery
- Useful for loading TF scripts: empty pattern, Startup enabled, command `#load myconfig.tf`

### F8 Highlighting
- Toggle highlighting of lines matching any action pattern
- Useful for debugging patterns without running commands

## Auto-Login

Three modes (configured per-world):
- **Connect** (default): Sends `connect <user> <password>` 500ms after connection
- **Prompt**: Sends username on first telnet GA prompt, password on second
- **MOO_prompt**: Like Prompt, plus username again on third prompt
- Only triggers if both username AND password are configured
- Auto-answered prompts are cleared and not displayed

## Spell Checking

- Uses system dictionary at `/usr/share/dict/words` (fallback: american-english, british-english)
- Words only checked when "complete" (followed by space/punctuation)
- Contraction support (didn't, won't, I'm)
- `Ctrl+Q` cycles through suggestions (Levenshtein distance, max 3)
- Misspelled word positions cached between keystrokes to prevent flickering

## Command Completion

- `Tab` when input starts with `/` cycles through matching commands
- Matches internal commands and manual actions (actions with empty patterns)
- Case-insensitive, arguments preserved when cycling

## Filter Popup (F4)

- Small popup in upper right corner
- Output shows only matching lines (case-insensitive substring)
- ANSI codes stripped for matching, preserved in display
- PageUp/PageDown scrolls filtered results
- `Esc` or `F4` closes

## Popup Definitions

### World Selector (`/worlds`)
- List of all worlds: name, hostname, port, user
- Filter box, current world marked with `*`, connected worlds in green
- Buttons: Add, Edit, Connect, Cancel

### World Editor (`/worlds -e`)
Per-world: name, hostname, port, user, password, SSL, log file, encoding, auto login type, keep alive type/cmd

### Global Settings (`/setup`)
More mode, spell check, temp convert, world switching, show tags, input height, console theme, GUI theme, console mouse, ZWJ, ANSI music, TLS proxy

### Web Settings (`/web`)
WebSocket (secure/non-secure), HTTP/HTTPS, TLS cert/key, allow list

### Actions List (`/actions`)
- List with enable status, name, world, pattern
- `Space` toggles enable, `Enter` edits, `A` adds, `D` deletes
- Filter box with `F` or `/`

### Action Editor
Fields: Name, World, Match Type (Regexp/Wildcard), Pattern, Command (multiline), Enabled, Startup

## ANSI Music

- Format: `ESC [ M <music_string> Ctrl-N` or `ESC [ N <music_string> Ctrl-N`
- Music string uses BASIC PLAY syntax: notes (A-G), octave (O, <, >), tempo (T), length (L)
- Web: Web Audio API with square wave oscillator
- WebView GUI: rodio library
- Console: stripped from display (no audio)

## GMCP Media (MCMP Protocol)

- Packages: `Client.Media.Default`, `Client.Media.Play`, `Client.Media.Stop`, `Client.Media.Load`
- F9 master mute switch (starts muted)
- Console playback uses ffplay or mpv (auto-detected)
- Media cached in `~/.clay_media_cache/`
- Per-world tracking: switching worlds stops/restarts media

## Crash Handler

- On panic, saves state and attempts restart
- Maximum 2 restart attempts
- TCP connections preserved (same as hot reload)
- Crash count cleared after first successful user input
- Uses `--crash` flag for crash restarts
