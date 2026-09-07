# Interface Overview

## Screen Layout

Clay's terminal interface is divided into three main areas, stacked vertically, plus an
optional status line that appears once a world starts reporting data (see **Status Display**
below):

1. **Output Area** - Takes most of the screen, displays MUD text
2. **Status Line** *(only once a world has data)* - Single line of MUD-reported stats, sits
   directly above the separator bar
3. **Separator Bar** - Single line with status information
4. **Input Area** - Bottom section for typing commands (1-15 lines)

Example separator bar format:

    __________ * HeartOfGold     (Activity: 2)              14:30
    [status]   [world name]      [activity]                 [time]

When paused or scrolled back:

    More: 1234 * HeartOfGold     (Activity: 2)              14:30
    Hist:  500 * HeartOfGold     (Activity: 2)              14:30

### Output Area

The output area displays text from the MUD server:

- **ANSI color support**: Full 256-color and 24-bit true color
- **Unlimited scrollback**: Output is stored in memory (grows with available RAM)
- **Word wrapping**: Long lines wrap intelligently, preserving ANSI codes
- **MUD tags**: Optional display of channel tags and timestamps (toggle with F2)

### Separator Bar

The separator bar (made of underscores) contains several indicators:

| Position | Component | Description |
|----------|-----------|-------------|
| Left (10 chars) | Status | `More: XXXX` when paused, `Hist: XXXX` when scrolled, underscores otherwise |
| After status | Connection | Green ball (🟢) + world name when connected |
| Center | Activity | `(Activity: X)` count of worlds with unseen output |
| Right | Time | Current time in HH:MM format (cyan) |

**Status indicator formatting:**
- Numbers up to 9999 shown as-is
- 10000+ formatted as "10K", "999K"
- 1000000+ shown as "Alot"

### Input Area

The input area is where you type commands:

- **Prompt display**: Server prompts (detected via telnet GA/EOR) shown in cyan
- **Multi-line support**: Resize with `Alt+Up/Down` (1-15 lines)
- **Cursor**: Standard text cursor with left/right movement
- **Spell checking**: Misspelled words highlighted in red

## Status Display

Above the separator bar, Clay can show a single line summarizing the current world's GMCP
`Char.*` and MSDP data — HP/mana-style vitals and whatever else the MUD reports. `/stats`
(see the **Commands** chapter) shows the same data in full; the web interface, the native
WebView GUI, and the Android app show it as a collapsible panel instead (see the **Web
Interface** chapter).

- **Appears and disappears with data, never with a setting.** There is nothing to turn this
  on or off: the line appears the moment the world's first stat arrives and disappears again
  on disconnect. A world that never sends anything relevant — most MOO and MUSH servers,
  which generally implement neither GMCP `Char.*` nor MSDP — simply never shows a status
  line. That is correct behavior, not a fault.
- **Fixed height once shown.** The line goes from 0 to 1 row exactly once per connection and
  stays 1 row for the rest of it, even as individual fields come and go — a MUD dropping or
  adding a stat mid-session never resizes the output area or disturbs scroll position.
- **As many entries fit.** A value paired with its maximum by name (e.g. `hp`/`maxhp`) shows
  as `key current/maximum`; anything else shows as `key value`. Entries are packed left to
  right until the line runs out of width, with a trailing `+N more` when some don't fit —
  `/stats` lists everything regardless of width.
- **Field names are exactly what the MUD sends**, with no reformatting — there is no fixed
  schema, so Clay pairs a value with its maximum by name shape rather than by a lookup table
  of known field names. This is also why an unrecognized field is never hidden: it just shows
  as a plain `key value` entry.

## More-Mode Pausing

When a lot of output arrives at once, Clay pauses to let you read. The separator bar displays `More: XXXX` (where XXXX is the number of pending lines) in red, indicating that output is being held back. This prevents fast-scrolling text from flying past before you can read it.

**Trigger conditions:**
- Automatic: After (screen height - 2) lines of output without user input
- Manual: When you scroll up with PageUp

**Controls when paused:**
- `Tab` - Release one screenful of pending lines
- `PageDown` - Release all pending and scroll to bottom
- `Escape` then `j` - Jump to end, release all pending
- `Enter` - Sends your command but does NOT release pending lines

**Pending line counter:**
- Shows `More: XXXX` in the separator bar
- Counts lines waiting to be displayed

## Scrollback Navigation

Use PageUp/PageDown to scroll through output history:

- **PageUp**: Scroll back in history (enables more-pause mode)
- **PageDown**: Scroll forward (unpauses if you reach the bottom)
- **Escape+j**: Jump to the bottom and release all pending

When scrolled back, the separator shows `Hist: XXXX` indicating lines from bottom.

## Themes

Clay supports light and dark themes for both console and GUI:

**Console themes** (set in `/setup`):
- Dark (default): Light text on dark background
- Light: Dark text on light background

**GUI themes** (set in `/setup`):
- Dark (default): Dark mode interface
- Light: Light mode interface

## Colored Square Emoji

Clay renders colored square emoji (🟥🟧🟨🟩🟦🟪🟫⬛⬜) with proper colors:

- **Console**: Converted to ANSI true-color block characters (██)
- **Web/GUI**: Native rendering with correct colors

This ensures consistent color display across all interfaces.

## Display Width Handling

Clay correctly handles:
- **Wide characters**: CJK characters, emoji (2 columns wide)
- **Zero-width characters**: Combining marks, zero-width spaces
- **Mixed content**: Cursor positioning works correctly with any mix

\newpage

