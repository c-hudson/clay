# Shell Worlds

A **Shell** world is a world whose "connection" is a terminal running on the computer
where Clay itself runs. Set a world's **Type** to **Shell** and connecting it opens your
login shell — you can run `vi`, `top`, `ssh`, `git`, anything — right where a MUD's output
would be. It is a full terminal, not a command box.

## Creating one

In the world editor, choose **Type: Shell**. The only other field is **Command**:

- Leave it blank to run your login shell (`$SHELL -l`; on Windows, the command
  interpreter in `%COMSPEC%`, usually `cmd.exe`).
- Set it to run something else instead, for example `top`, `ssh server`, or
  `tmux attach`. It runs through your shell (`$SHELL -lc "<command>"`).

Then connect the world the way you connect any other — `/worlds <name>`, the Connect
button, or switching to it. Disconnecting (`/dc`, the Disconnect button) ends the shell;
when the program exits on its own, the world shows `[Clay: shell exited (code N)]` and
stops. While the shell is not running, the world behaves normally again, so you can type
`/connect` to start a fresh one.

## Keys

While a shell world is on screen, **every key goes to the terminal** — including Tab,
the arrow keys, Ctrl+C, Ctrl+Z, Ctrl+D, function keys and Page Up/Down. The one
exception is the keys bound to **switching between worlds**:

- Next / previous active world (`Esc-}` / `Esc-{` by default)
- Next / previous world of all (`Shift-Up` / `Shift-Down`)
- The world with activity (`Esc-w`)
- Next / previous connected world (`Esc-Left` / `Esc-Right`, `Alt-Left` / `Alt-Right`)
- The recent-worlds list, and switch-to-previous / switch-forward

Those still switch worlds. Everything else — all other TinyFugue keys, triggers, Actions
and macros — does not apply inside a shell world; the terminal gets the keystrokes raw.
Pressing **Escape** sends Escape to the terminal immediately (so `vi` works), and still
begins a world-switch chord, so a slow "Escape then `}`" also switches.

Because Clay's own commands are keystrokes to the shell, to run a Clay command (or quit
Clay) from a shell world, switch to another world first.

The mouse is passed to a program that asked for it (like `vi` or `tmux`): the wheel on
every interface, and clicks and drags in the console and the web/WebView client. There is
no separate scrollback for a shell world, so the wheel does nothing at a plain prompt.

## In a browser

The web client forwards what it can, but a browser keeps a few shortcuts for itself —
Ctrl+W, Ctrl+T, Ctrl+N and the like cannot be captured in a browser tab. For a shell
world where you need every key, use the desktop GUI, the WebView, or the console. Typing,
including with an input method (IME) and on a phone's on-screen keyboard, is sent to the
terminal as you type.

## What a shell world does not do

A shell world is deliberately plain: its output is **not** logged, archived, searched
with `/recall`, matched by triggers or Actions, highlighted, or kept in history. Clay
tracks only **activity** for it — and only when a printable character actually appears on
the screen. Moving the cursor around or redrawing with the same text does not light the
activity indicator; new text does. Blinking text is not shown.

## Security

A shell world hands a shell **as the user running Clay** to anyone who can connect to
that Clay — the local console, the desktop GUI, and every authenticated remote client
(the web client, Android, a remote console over SSH). This is the same reach the `/sh`
command already gives those clients, but it is worth stating plainly: **do not create a
shell world on a Clay you expose to a network you do not trust.** Shell worlds are not
allowed in multiuser mode at all, since there the shell would run as the server's user on
behalf of whichever account owns the world.

Surviving a reload: on Linux, a `/reload` keeps shell worlds running — the program you
left open (your editor, `top`, an `ssh` session) is still there afterward, and repaints
itself (that repaint does not count as activity in a shell world you are not looking
at). On macOS and Windows a reload closes shell worlds (the program ends), like other
non-MUD worlds. Note that, because every key goes to the terminal, you cannot type
`/reload` *in* a shell world; switch to another world first (or Clay reloads on its own
after an update).
