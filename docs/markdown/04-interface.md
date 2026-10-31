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

The separator bar contains several indicators. Its gaps are spaces on the status bar
colour, or underscores with Settings > Separator Style set to TinyFugue (see **Settings**):

| Position | Component | Description |
|----------|-----------|-------------|
| Left (10 chars) | Status | `More: XXXX` when paused, `Hist: XXXX` when scrolled, underscores otherwise |
| After status | Connection | Green ball (🟢) + world name when connected |
| Center | Activity | `(Activity: X)` count of worlds with unseen output |
| Right, before the time | Notes | 📝 when the current world has notes (`/note`) |
| Right | Time | Current time in HH:MM format (cyan) |

**Status indicator formatting:**
- Numbers up to 9999 shown as-is
- 10000+ formatted as "10K", "999K"
- 1000000+ shown as "Alot"

### Input Area

The input area is where you type commands:

- **Prompt display**: Server prompts (detected via telnet GA/EOR, or by timing on MUD - Timed Prompt worlds) shown in cyan
- **Multi-line support**: Resize with `Alt+Up/Down` (1-15 lines). In the GUI and the web client you can also drag the line above the input up or down with the mouse; the console has no drag, only the keys. Either way the height is kept (it is that interface's Input height setting: the console, phones, tablets and desktop clients each have their own) and comes back on the next start and after `/reload`
- **Lines sent stay on screen**: as in TinyFugue, a line you send stays in the input area (its text alone; the prompt belongs to the line you are typing, which shows the current world's), and the next one starts on the row below. Once the area is full, the oldest row scrolls off the top, and the line being typed takes the rows it needs from them. A line typed at a password prompt leaves an empty row. `Ctrl+L` (Redraw) clears them. Turn **Clear Input on Send** on in Settings (TF's `/cleardone on`) to have Enter clear the input instead. On a phone the input always clears
- **Cursor**: Standard text cursor with left/right movement
- **Spell checking**: Misspelled words highlighted in red

### Split Output

`/split` splits the output area in two, one above the other, so you can watch two worlds
at once. It works the same in every interface: the console, the SSH console, the GUI, the
web page and the Android app.

    Rome: the forum is busy today...
    __________ ● Rome________________________________________________
    You are standing in the HeartOfGold's galley...
    __________ ● HeartOfGold     (Activity: 2)                   14:30
    > _

- **`/split`** opens a pane above the output. The world you are in stays in the bottom
  pane and keeps the focus; the new top pane shows the next world in order (the next
  connected one alphabetically, else the next of all worlds). **`/split <world>`** shows
  that world instead (an unknown name is an error); while already split, it puts that
  world in the pane without the focus.
- **`/unsplit`** closes the pane that does *not* have the focus.
- Neither command connects or disconnects anything.
- **The focus** is the pane that gets what you type, your commands and your keys: `/world
  X` switches the focused pane, Page Up scrolls it. Move it with **Shift+Tab** in every
  interface, or, in the GUI, the web page and the Android app, by clicking (tapping) the
  other pane, as long as the click doesn't select text or follow a link.
- **The bars.** The bar between the panes belongs to the top pane and shows only its world's
  name and connection ball. The usual bar under the bottom pane belongs to the bottom one,
  with everything it always shows; its activity count leaves out both worlds on screen. The
  focused pane's bar keeps the bar colour, the other one turns grey.
- **Both panes stay live.** Each takes its world's output under the usual rules: more-mode
  pauses each at its own height, ▶ marks what arrived unseen, and neither counts as activity
  while it is on screen.
- **The switch keys** (`Esc-{`, `Esc-}`, `Esc-w`, `Esc-Left`/`Right` and the rest) skip the
  world in the other pane, so the two never show the same world by accident. `/world X`
  (or `/split X`) may show one world in both: in the GUI and the web page each pane then
  scrolls on its own; in the console the pane without the focus keeps showing the newest
  lines while the focused one is scrolled back.
- A split survives `/reload` and `/update`, and a GUI restart. Deleting a world shown in a
  pane closes the split.

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

With Archive Input/Output on, PageUp at the top of what's in memory goes on into the
archive (`scrollback.db`), 500 lines at a time; archived lines are marked 🛢️.

A GUI, a browser, the Android app or a console attached to another Clay scrolls back the
same way. When it connects it downloads only the newest lines (the **Remote Lines**
setting's worth), never the archive. Scrolling or paging up past the oldest line it holds
(the mouse wheel, a drag, PageUp) asks the server for more, 500 lines at a time: first the
rest of the server's memory, then its archive. Each batch is asked for only when you reach
it. A browser page or the app keeps 5000 lines on the page at once; going further back
drops the newest from the page, and they come back as you scroll down again.

In the GUIs (`--gui` and `--web-gui`) and in a desktop browser, the output also has a
scroll bar along its right edge. Drag its thumb to move through the scrollback, or click
the bar beside the thumb to jump there (in Firefox a click there moves one page instead).
Dragging up pauses output just as PageUp does, and dragging back to the bottom follows new
output again. The thumb takes the theme's muted text colour and brightens under the mouse.
The bar covers what the GUI holds, and grows as scrolling back fetches more. Phones and
tablets keep the platform's own scroll bar.

## Themes

Clay supports light and dark themes for both console and GUI:

**Console themes** (set in `/setup`):
- Dark (default): Light text on dark background
- Light: Dark text on light background

**GUI themes** (set in `/setup`; the web and GUI share them):
- Dark (default): the original dark interface
- Light: the original light interface
- Dark - New: slate blues built from the separator bar's colour, with a warm orange accent and higher-contrast text. On a phone (the web page's phone layout and the Android app), the buttons and font slider in the row under the separator bar, and the input area's background and the line above it, keep the original Dark theme's colours.
- Light - New: a warm off-white interface with charcoal text and a terracotta accent

Each is a `[theme:name]` section in `~/.clay/theme.dat` (`dark`, `light`, `dark-new`, `light-new`) and can be edited in the theme editor.

The desktop GUI, the web page and the Android app draw the same chrome from whichever
theme is chosen: settings grouped in rounded cards, a sidebar of icon-and-label tiles
(icons only on a phone), the accent colour on the primary buttons and switches, world
tabs sitting on the ribbon with the current one raised, and the input bar on the surface
colour under a thin line. Whether a theme counts as light is decided from its background
colour, not its name, so a custom light theme gets the light treatment too. The theme,
keybind and action editor pages follow the theme as well.

## Importing a Picture

A picture can go into what you type as a link: Clay uploads it to a free picture host and
puts the link into the input line at the cursor. Text already in the line stays as it is
(a space is added where the link would touch a word), and the cursor is left at the start
of the link.

- **Console and remote console**: drag the picture onto the terminal. The terminal hands
  Clay the file's path, so Clay asks first: "Upload cat.png (2.1 MB) to catbox.moe?" (a
  pasted path to a picture looks the same as a dropped one, and uploads are public). No
  is the default. Anything that isn't a picture types its path as before.
- **Web page**: drag the picture onto the page, or Menu > Import Picture.
- **Android app**: Menu > Import Picture, then choose a gallery, photos or files app.
- **GUIs**: Menu > Import Picture (a file dialog), or drag the picture onto the window.

Several pictures at once go up one after another, and their links go in together. The
upload always runs on the Clay server (the main instance), whose **Picture Host** setting
(Settings > General) chooses the host: catbox.moe (the default; kept until nobody has
viewed it for two years), sxcu.net (pictures only, three uploads a minute), x0.at (kept
3 to 100 days, shorter for big files) or tmpfiles.org (deleted after two days; its link
opens tmpfiles' page with the picture and a Download button, not the picture alone). Each
has a "+ URL Short" choice that runs the link through `/url`'s shortening. Off turns
uploads off.
Pictures over 32 MB aren't uploaded. Anyone with a link can see the picture, and these
hosts don't let you delete it.

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

