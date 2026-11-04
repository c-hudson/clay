# Accessibility

Clay works with screen readers in every interface: the console (with a terminal screen
reader such as NVDA, Orca, VoiceOver or Speakup), the GUI, the web page and the Android
app (with TalkBack).

A screen reader is a different thing from Clay's own text-to-speech (`/tts`, the TTS
setting). Text-to-speech is Clay reading the game's lines aloud in a voice it chooses. A
screen reader is your own program reading the whole of Clay, menus, settings and dialogs
included, in your own voice and speed, often on a braille display too. Clay's part is to
tell the screen reader what is on the screen, and to read new lines out once.

## Screen Reader mode

**Settings > Screen Reader** turns it on for the interface you set it in; each interface
keeps its own, like Text Reveal and TTS. When it is on:

- **New lines are read out once**, as they appear: a line that arrives, or a page that
  more-mode lets through. The text alone is read: no colour codes, no MUD channel tags, no
  ▶ marker. Gagged lines are never read. Redrawing the screen (switching worlds, resizing,
  a filter) reads nothing, so you hear each line once.
- **Text Reveal is off** in that interface: a line that is still being typed on can't be read.
- **That interface's TTS is quiet** for incoming lines, so nothing is read twice; `/say`
  and `/tts test` still speak.
- **The focus stays where you put it.** The web page and the Android app no longer pull
  the focus back to the input line when you move around the page.

Where Clay can tell a screen reader is running, an interface that never chose turns the
mode on by itself, once: the GUI when a screen reader starts using it, and the Android app
when TalkBack is on. Turn it off in Settings and it stays off.

**Settings > MUDs: Screen Reader** tells the games you connect to that you use a screen
reader (the screen-reader flag of the MTTS terminal-type report), so a game that knows the
flag can send simpler text: fewer ASCII maps, no art. It is one setting for every
interface and takes effect at the next connect.

## In each interface

**The console.** Screen Reader mode leaves the clock out of the separator bar (a clock
that changes every minute repaints the bar, and a terminal screen reader reads the change
out), turns Text Reveal off, and keeps the terminal cursor on the input line, where you
type, at the end of every screen update. Settings are in `/setup`. The console attached
to another Clay (`--console`, `--ssh`) has its own setting, kept by that Clay.

**The GUI.** The output is a log whose lines the screen reader can review, the input line
is a text field named "Command" holding what you have typed, and the buttons, tabs and
menu items have names. New lines are read from a live region, once.

**The web page, the WebView GUI and the Android app.** The output is a log, the input is
named "Command" and described by the prompt, and every button, menu, tab, switch and
dialog is reachable with Tab and named: Enter or Space presses it, arrow keys move through
an open menu and Escape closes it. Dialogs open with the focus inside and give it back
when they close. In Screen Reader mode:

- **F6** and **Shift+F6** move between the input line, the output, the world tabs, the
  status bar's world name and, when it is open, the map.
- Phones and tablets keep the system's own dropdowns, which a screen reader can work,
  instead of Clay's drawn ones.

## Settings

| Setting | What it does | In `settings.dat` |
|---------|--------------|-------------------|
| Screen Reader | This interface's Screen Reader mode | `screen_reader` (the console), `screen_reader.<interface key>` (the others) |
| MUDs: Screen Reader | Tell games a screen reader is in use (MTTS) | `announce_screen_reader` |
