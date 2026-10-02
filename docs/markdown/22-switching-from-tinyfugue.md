# Switching from TinyFugue

Clay can take TinyFugue's place: it reads your `.tfrc`, takes TF's command line,
and runs TF's macros, triggers, hooks, key bindings and status line. A Clay
started without a `.tfrc` behaves as it always has. This chapter is
for a TinyFugue user moving over; the command reference is "TinyFugue Commands".

## Starting Clay

Run `clay` the way you ran `tf`:

```
clay                  Load your .tfrc, connect nothing
clay mymud            ...then connect the world "mymud"
clay mud.org 4000     ...then connect a temporary world for that address
clay -f               Don't load a .tfrc
clay -f~/other.tf     Load ~/other.tf instead
clay -c'/echo hi'     Run a command after the .tfrc
clay -L/usr/share/tf5/tf-lib    Use that library directory
clay -l mymud         Connect without logging in   (-q: a quiet login)
```

At startup Clay loads, as TF does, the first of `~/.tfrc`, `~/tfrc`,
`./.tfrc` and `./tfrc` - with `%TFLIBDIR/local.tf` before it and
`~/.tinytalk` after it - and says `% Loading commands from <file>.` for each.
Two things differ, by design:

- **`-v` shows Clay's version.** Clay has no non-visual mode: its console always
  draws its windows, and `/visual off` in a `.tfrc` only says so.
- **Nothing connects on its own.** TF connects to the first world it is
  given; Clay connects only a world you name on the command line (`clay mymud`)
  or with `/connect`.

A hot reload (`/reload`) or a crash restart keeps everything TF knew -
macros, hooks, bindings, variables, running `/repeat`s, the status line - and
does not run the `.tfrc` again.

## Your worlds

`/addworld` lines in the `.tfrc` become worlds in Clay's world list, saved with
everything else: you will see them in `/worlds` and every client. Running the
`.tfrc` again changes only what a line says, and says nothing when nothing
changed - a world you've adjusted in Clay's world editor keeps its other
settings. Like TF, Clay warns when a file holding passwords can be read by others.

A world's TF type (`-T`) is kept and shown in the world editor. `lp`, `lpp`
and `diku` worlds log in by sending the character and password as two lines;
`tiny` worlds send `connect <character> <password>`. `%login`, `/login off`,
`/connect -l` and the LOGIN hook work as in TF. `/saveworld`, `/loadworld`,
`/purgeworld`, `/unworld` and TF's `/listworlds` are there too.

`/connect` is TF's: `/connect mymud`, `/connect -b mymud` (in the background),
`/connect mud.org 4000` (a temporary world, gone when it disconnects). Clay's
own command for attaching to another Clay is **`/server`**.

## What changes as you type

As in TF, a line you type is not `%`-expanded (`%sub` is off): `/echo %{hp}`
prints `%{hp}`. `/sub on` and `/sub full` work as in TF. As in tf 5.0b8, `//cmd`
runs `/cmd` and `///text` sends `/text` to the world; `^old^new` repeats the last
line with `old` replaced. Text inside macros is expanded exactly as TF expands it.

## Settings that are Clay's own

Some TF variables are Clay settings, so setting one changes Clay itself - in
every interface - and Clay's settings show through them:

| Variable | Clay setting |
|----------|--------------|
| `%more` | more-paging |
| `%wrapspace` | the indent of wrapped lines |
| `%isize` | the height of the input area |
| `%insert` | insert/overstrike |

They are not saved as TF variables; Clay saves the settings themselves.

## The status line

Clay shows its own status bar until your `.tfrc` (or anything else) changes
TF's status line - `/status_add`, `/status_rm`, `/status_edit`, `/clock`, or a
`status_*` variable. (Setting `%clock_format` alone changes the clock on Clay's own
bar.) From then on Clay draws TF's, with tfstatus.tf's formats:
in the console, in the web and desktop clients (next to their menu, notes and
font controls), and on Android, each laid out to its own width.

## New text when you switch worlds

`%textdiv` works as in TF once your `.tfrc` sets it: switching to a world shows a
`%textdiv_str` divider (`=====`) between the text you had seen there and what arrived
since (`on`), shows one even when nothing arrived (`always`), or takes the old text
out of view (`clear`) - in the console and in the web, desktop and Android clients.
Each of those keeps track of what it has shown you, and the divider is gone once you
switch away. While `%textdiv` is unset, Clay marks new text its own way (the ▶ new
line indicator, when it's turned on); `/unset textdiv` brings that back.

## History

`/recall` and `/quote #` read the same history TF keeps. A line a trigger gives the
`G` (nohistory) attribute - or that `/echo -aG` or `/substitute -aG` shows - is
displayed as usual but stays out of it, and out of Clay's scrollback archive.
`/recordline` puts a line into the world (`-w`), local (`-l`), global (`-g`, the
default) or input (`-i`) history without showing it, as `savehist.tf` does.
`/recall -a<attrs>` shows lines without those of their own attributes (`-ag` shows
gagged ones), and `/echo -ag` keeps a line there without showing it.

## Connection hooks

CONNECT, DISCONNECT, CONFAIL and ICONFAIL get TF's arguments: CONNECT the world and,
for TLS, its cipher (`mud TLS_AES_256_GCM_SHA384`); DISCONNECT the world alone when the
server closed the connection, `<world> recv <error>` when a read failed, and
`<world> <address> <port>: <reason>` for a connect that failed (ICONFAIL for each
address of a name that failed before the next was tried). Clay prints its own
messages ("Connection closed by server.", "Disconnected.", "Connection failed: ...")
rather than TF's, and a gagged hook hides them. `/dc` fires no DISCONNECT, as in TF.

## Help

`/help` answers from Clay's own help, and for a topic Clay doesn't cover - `/help
intro`, `/help special variables`, `/help kecho` - shows TinyFugue's own help
text when `tf` is installed (Clay reads its `tf-help` file).

## What is different, or not there yet

- **Keys**: TF's defaults, except `^Q` (spelling suggestions) and `^R` (hot
  reload); `Tab` pages through held output first. See "Keyboard Shortcuts".
- **`/histsize`**: Clay keeps a world's whole output - archived, searchable with
  `/recall -D` - and the whole input history, so there is no size to set.
- **`/repeat` and `/quote`** without `-w` stay with the world they started in.
- **`/sh` from the web or desktop client** runs in the background and shows its
  output when done; an interactive shell needs Clay's own console.
- **`%wrapsize`**: until a script sets it, Clay wraps output at the window's edge;
  once set, output wraps there, in every interface.
- **Mail checking** happens only once your setup asks for it - a MAIL hook, `@mail`
  on a status line you've changed, or `%TFMAILPATH`/`%MAIL` set in your `.tfrc` -
  not merely because `$MAIL` is in the environment.
- **`/recordline -i`** adds to the console's Up/Down history; the web, desktop and
  Android clients keep their own.
- **`%textdiv` "clear"** leaves a screen-high blank above the new text: scrolling back
  passes it before the old text, until you switch away.
- **Not there**: the PROXY hook, since Clay doesn't connect through a proxy
  (`%proxy_host`).

And everything Clay adds stays available beside TF's ways: the web, desktop and
Android clients, actions, the world editor, spell checking, the scrollback
archive, Slack and Discord worlds.
