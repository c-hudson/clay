# TinyFugue Commands

Clay includes a TinyFugue (TF) 5.0 compatibility layer. TF veterans can bring
their existing triggers, macros, and keybindings over largely unchanged.
Commands use the `/` prefix, exactly like Clay's own commands (there is no
separate `#` command prefix — see "A note on `#`" below).

## Variables

### /set / /unset
Set or remove global variables.

```
/set varname value      Set variable
/unset varname          Remove variable
```

### /let
Set a local variable (within macro scope). Unlike `/set`, the value is kept
exactly as typed — leading/trailing spaces are **not** trimmed.

```
/let temp_value 100
```

### /setenv
Export a variable to the environment, for `/sh` and `/quote`.

```
/setenv MY_VAR
```

### /listvar
List variables matching a pattern.

```
/listvar              List all variables
/listvar hp*          List variables starting with "hp"
```

## Output Commands

### /echo
Display a local message (not sent to the MUD). Inside a macro body the text
is substituted first; a line you type is not (see "Typed Lines").

```
/echo Hello, world!
/def hp = /echo Your HP is %{hp}
/echo -aBu Bold and underlined
/echo -p Your @{Cred}HP@{n} is low
```

Attributes are TF's: letters run together (`-aBu`), `C<color>` for a color (`@{BCgreen}`
is bold green), `h` for `%hiliteattr`; inline `@{...}` codes add up until `@{n}`.
`/echo -ag` keeps the line in the history without showing it.

### /send
Send text to the MUD, bypassing macro/alias expansion.

```
/send look
/send -w MyMUD say hello    Send to a specific world
```

### /beep
Ring the terminal bell.

```
/beep
```

### /recordline
Put a line into a history without showing it - `/recall` finds it there, and it is
never logged or archived. `-w[<world>]` the world's history, `-l` local, `-g` global
(the default), `-i` input (which also feeds the console's Up/Down history);
`-t<time>` gives it a time (seconds, as `/recall -t@` shows), `-a<attrs>`
attributes, `-p` interprets `@{...}` inline.

```
/recordline -i north
/recordline -w -t1696000000 an old line
```

A line with the `G` (nohistory) attribute is the opposite: shown, but never in the
history - `/def -aG -t"*whispers*" quiet`, `/echo -aG`, `/substitute -aG`.

### /quote
Generate lines from a file (`'`), a shell command (`!`, standard error
included), a TF command (`` ` ``) or `/recall` (`#`), put an optional
<pre> before and <suf> after each, and send, echo or run them, as TF does:
sent when there is no <pre>, run as commands (never expanded) when there is.
Unless `-S`, a quote is a background process: one line every `%ptime` (1
second; `-<time>` to change it, `-0` for all at once), or one per prompt with
`-P`. `/ps` lists it and `/kill` stops it.

```
/quote -S '"/tmp/lines.txt"        Send each line of the file, now
/quote say '"/tmp/lines.txt"       Run "say <line>" for each line, one a second
/quote think `"/version"           Send "think <the /version text>"
/quote -S -decho !ls -la           Show the output of a shell command
/quote -P /send `/echo n%;/echo w   Send "n", then "w" at the next prompt
```

## Processes

`/repeat` and background `/quote`s are processes. `/repeat -30 5 /echo hi`
runs `/echo hi` five times, the first time 30 seconds from now (`-n`: now;
no `-<time>`: `%ptime`). `-P` runs on prompts; with `%lpquote` on, every
process does. `/ps` lists them in TF's table (`-s` just the pids, `-r`/`-q`
only repeats/quotes), `/kill <pid>` stops one.

## Shell

`/sh <command>` runs a command with /bin/sh, and plain `/sh` an interactive
`%SHELL`, on the terminal: Clay's screen is put away while it runs and comes
back after a keypress (`%shpause`). `%?` is its exit status. `^C` reaches
only the shell. From the web/GUI client, or a Clay with no console, a command
runs in the background and its output is shown when it finishes. `/suspend`
stops Clay as `^Z` does. TF's `/psh` comes with `/require psh.tf`.

## Prompts

`/prompt [-a<attrs>] [-p] <text>` (or `prompt(<text>)`) makes <text> the
current world's prompt. A PROMPT hook that matches takes the prompt over and
shows its own:

```
/def -h"PROMPT *> " catch_prompt = /test prompt({*})
```

## The Status Line

TF's status line is rows of fields, each `name[:width[:attributes]]`:
padding (`:2`), an internal state (`@more @world @read @active @log @mail
@clock`, shown by the expression in `%status_int_<name>`), a variable (shown
by `%status_var_<name>`, else its value) or a "literal". One field may have
no width and takes what the others leave; a negative width right-justifies.

```
/status_add -A@world hp:4         Show %hp after the world name
/status_rm @mail                  Remove the mail field
/status_edit @log:1               Make the log field one column
/set status_height=2              Two rows (status_add -r1 ... fills row 1)
/clock %I:%M                      A 12-hour clock
/status_defaults                  TF's own fields again
```

Clay shows its own status bar until a script changes the status line (any of
these commands, or setting a `status_*` variable). From then on TF's is drawn
instead - in the console, the web/GUI/Android client (beside its menu, notes
and font controls) and the SSH console, laid out to each one's width.
`status_fields([row])` lists a row's fields; `/status_save` and
`/status_restore` keep and bring back row 0.

## Visual Mode

Clay's console always draws its windows: there is no non-visual mode.
`/visual off` (or `/set visual=off`) answers `% Clay's console has no
non-visual mode.`, and `%visual` always reads `on`.

## New Text on a World Switch

Once a script sets `%textdiv`, switching to a world shows TF's divider:

```
/set textdiv=on        %textdiv_str (=====) between what you'd seen and what's new
/set textdiv=always    ...even when nothing new arrived
/set textdiv=clear     the old text taken out of view instead
/set textdiv=off       nothing
/unset textdiv         back to Clay's own ▶ markers
```

The console, the web/GUI/Android client and the SSH console each track what they
have shown you; the divider goes when you switch away.

## Expressions

### /expr
Evaluate and display an expression's result.

```
/expr 5 + 3           Displays: 8
/expr strlen("hello") Displays: 5
```

### /test
Evaluate an expression, return its value, and set `%?` — unlike `/expr`,
doesn't display the result automatically.

```
/test 5 > 3           Returns: 1
/test hp < 50         Returns: 1 or 0
```

### /eval
Run one more substitution pass over its argument (`%vars`, `$[...]`,
`$(...)`), then execute the result as a command — this is how you run a
command whose *name* is held in a variable, since an ordinary `/name` line
only ever substitutes once, before `/name` is even identified.

```
/set cmdtail=echo hi
/eval /%cmdtail        Prints: hi
/set v=7
/eval /echo v=%v       Prints: v=7
/eval -s0 /echo v=%v   Prints "v=%v" literally (no extra substitution pass)
```

### Expression Operators

| Category | Operators |
|----------|-----------|
| Arithmetic | `+` `-` `*` `/` `%` |
| Comparison | `==` `!=` `<` `>` `<=` `>=` |
| Logical | `&` `\|` `!` |
| Regex / glob | `=~` `!~` `=/` `!/` |
| Ternary / comma | `? :` `,` |

### Built-in Functions

| Function | Description |
|----------|--------------|
| `strlen(s)` | String length |
| `substr(s, start[, len])` | Substring |
| `strcat(s1, s2, ...)` | Concatenate strings |
| `tolower(s)` / `toupper(s)` | Case conversion |
| `replace(old, new, s[, count])` | Replace occurrences — **TF's own argument order** (see "Differences" below) |
| `rand([max])` / `rand(min, max)` | Random number |
| `time()` / `ftime([format])` | Current time / formatted time |
| `abs(n)` | Absolute value |
| `min(a, b, ...)` / `max(a, b, ...)` | Minimum / maximum |
| `ismacro(name)` | True iff a macro or builtin of this name exists |

See `reference/tf-engine.md` for the complete function list (string, math,
regex, world, file I/O, keyboard-buffer).

## Control Flow

### /if / /elseif / /else / /endif
Conditional execution — either a parenthesized expression, or a command
whose own return status (`%?`) is the condition.

```
/if (hp < 50) cast heal

/if (hp < 25)
  cast 'cure critical'
/elseif (hp < 50)
  cast heal
/else
  /echo HP is fine
/endif

/if /ismacro greet%; /then /echo already defined /else /echo not yet /endif
```

### /while / /done
While loop (parenthesized-expression or command-status form).

```
/while (count < 10)
  /echo Count: %{count}
  /set count $[count + 1]
/done
```

### /for / /done
For loop. The single-line form is TinyFugue's own (`min`/`max`, counting up
only); the multi-line `.../done` form with an explicit step is a Clay
extension.

```
/for i 1 10 /echo Number: %{i}

/for i 1 10 2        Clay extension: step by 2
  /echo Odd: %{i}
/done
```

### /break
Exit the nearest enclosing loop early (or `n` levels, with `/break n`).

```
/while (1)
  /if (done) /break /endif
/done
```

## Macros (Triggers)

### /def
Define a macro, with an optional trigger pattern.

```
/def name = command body

/def -t"pattern" name = command                      Trigger pattern

/def -t"* tells you: *" -mglob reply_tell = say Thanks, {1}!
```

**Options:**

| Option | Description |
|--------|-------------|
| `-t"pattern"` | Trigger pattern |
| `-mtype` | Match type: `simple`, `glob` (default), `regexp` |
| `-p priority` | Execution priority (higher = first) |
| `-F` | Fall-through (continue checking other triggers) |
| `-1` | One-shot (delete after firing) |
| `-n count` | Fire only N times |
| `-ag` / `-ah` / `-ab` / `-au` | Gag / highlight / bold / underline |
| `-E"expr"` | Conditional expression |
| `-c chance` | Probability (0.0-1.0) |
| `-w world` / `-T type` | Restrict to a specific world, or worlds of a type |
| `-hEVENT` / `-h"EVENT pattern"` | Hook event, with an optional argument pattern |
| `-b"key"` / `-B<name>` | Key binding, by raw sequence or by TF's named-key vocabulary |
| `-i` / `-I` | Invisible: hidden from `/list`/`/save`/`/purge` unless forced |
| `-q` | Quiet: doesn't count toward the BACKGROUND hook or `/trigger`'s return value |
| `name` omitted | Legal when `-t`/`-b`/`-B`/`-h` is given — the macro is addressed by number only |

### /undef / /undefn / /undeft
Remove macros. All three are silent on success.

```
/undef name           Remove by name
/undefn number...     Remove by sequence number (see /list, or %? after /def)
/undeft pattern       Remove matching trigger pattern
```

### /list
List defined macros.

```
/list                 List all
/list heal*           List matching pattern
```

### /purge
Remove macros matching a filter (same option grammar as `/list`); silent on
success.

```
/purge                Remove all
/purge temp_*         Remove matching name pattern
/purge -mglob temp_*  Same, explicit glob match
```

## Hooks

Define macros that fire on Clay/TF-internal events, the same way triggers
fire on MUD text:

```
/def -hCONNECT auto_look = look
/def -hDISCONNECT goodbye = /echo Disconnected!
```

**All 32 real TF events**, plus Clay's own `GMCP`/`MSDP` extras:

`ACTIVITY BAMF BGTEXT BGTRIG CONFAIL CONFLICT CONNECT DISCONNECT ICONFAIL
KILL LOAD LOADFAIL LOG LOGIN MAIL MORE NOMACRO PENDING PREACTIVITY PROCESS
PROMPT PROXY REDEF RESIZE SEND SHADOW SHELL SIGHUP SIGTERM SIGUSR1 SIGUSR2
WORLD` — see `/help hooks` or `reference/tf-engine.md` for what argument
text each one carries and the SEND/LOADFAIL hooks' special suppression
behavior. PROXY never fires (Clay has no `%proxy_host`).

The connection hooks carry TF's own arguments:

```
CONNECT     mud TLS_AES_256_GCM_SHA384            a TLS connection names its cipher
DISCONNECT  mud                                   the server closed the connection
DISCONNECT  mud recv Connection reset by peer     a read failed
CONFAIL     mud 127.0.0.1 4000: Connection refused
ICONFAIL    mud ::1 4000: Network is unreachable  one address of several failed
```

`/dc` fires no DISCONNECT. A gagged hook (`/def -ag -hDISCONNECT ...`) hides
Clay's own message ("Connection closed by server.", "Disconnected.",
"Connection failed: ...").

## Key Bindings

### /bind / /unbind
Bind key sequences to commands. `/bind key = cmd` is exactly `/def
-b"key" = cmd` — substitution happens fresh on every keypress, not once at
bind time.

```
/bind F5 = cast heal
/unbind F5
```

**Key names:** `F1`-`F20`; `^A`-`^Z` (Ctrl); `Esc-x`/`Alt-x`/`Meta-x`/`@x`
(all four equivalent, case preserved); `Ctrl-Up`/`Shift-Tab`/`Alt-Down`
(real terminal modifiers); `Up`/`Down`/`Left`/`Right`; `PgUp`/`PgDn`;
`Home`/`End`/`Insert`/`Delete`/`Tab`; and chords written back to back with
no separator (`^X^R`, `Esc-Left`). TinyFugue's own raw spellings (`^[b`,
`\033`, `\0x1B`, raw terminal escape sequences) are also accepted. See
`docs/markdown/07-keyboard-shortcuts.md` for Clay's complete default table.

### The key_<name> layer
`/def key_<name> = ...` redefines what a *named* physical key does,
independent of the raw byte sequence a particular terminal happens to send
for it (`key_f5`, `key_ctrl_left`, `key_esc_left`). Checked after any
`/bind` for the exact sequence, before Clay's own built-in action table.

## File Operations

### /load
Load a TF script file, with TinyFugue's own rules:

- A line starting with `;` or `#` is a comment — in the first column only
  (see "A note on `#`" below).
- A line ending in `\` continues on the next line, whose leading spaces are
  dropped; `%\` is a literal backslash.
- Every other non-blank line must be a `/command`. A line of plain text stops
  the load: `% <file>, line N: Invalid command. Aborting.`
- An indented line that starts a new command is probably missing the previous
  line's trailing `\`, so it draws `% <file>: line N: Warning: possibly missing
  trailing \` (naming the previous command's line) — and still runs.
- Errors are shown where they happen, as `% <file>, line N: <message>`
  (`lines A-B` for a command continued over several lines), and loading goes on.
- Lines are run exactly as written: `%var`, `$[...]` and `$(...)` are expanded
  only inside macro bodies and by `/eval`.
- `-q` drops the `% Loading commands from <file>.` message — for this file and
  every file it loads in turn.

A relative file name is looked for in the current directory (`/lcd`), then — for
a bare name with no `/` in it — in each directory of `%{TFPATH}` (space-separated;
write a space inside a name as `\ `), or in `%{TFLIBDIR}` when `%{TFPATH}` is
blank. `~` is your home directory and `~user` another user's.

```
/load scripts/my_triggers.tf
```

### /require
Like `/load` (same search), but a file that's already registered a `/loaded`
token isn't read again.

```
/require lisp.tf
```

### /save
Save macros to a file.

```
/save macros_backup.tf
```

### /lcd
Change local directory (affects relative `/load`/`/require`/`/save` paths).

```
/lcd /home/user/mud
```

## Loading TinyFugue's Own Library

TinyFugue ships a library of general-purpose macros (`lisp.tf`, `alias.tf`,
`kbfunc.tf`/`kbbind.tf`, `stdlib.tf`, and more) under `tf-lib/`. **Nothing
GPL-licensed ships with Clay** — Clay never bundles or vendors this library.
Instead, `/require somefile.tf` resolves a bare filename by searching
`%{TFPATH}`, or `%{TFLIBDIR}` when `%{TFPATH}` is blank, and `%{TFLIBDIR}` defaults to
`$TFLIBDIR` if set, else `/usr/share/tf5/tf-lib` if that directory exists —
the path the `tf5` distro package (Debian/Ubuntu: `apt install tf5`)
installs the real library to. On a machine with `tf5` installed,
`/require lisp.tf` (or any other library file) works exactly as it would
under real TinyFugue; on a machine without it, `%{TFLIBDIR}` is simply
unset and the `/require` fails to find the file, same as real TF would with
no library installed.

```
/require lisp.tf
/car (a b c)          =>  a
```

## Variable Substitution

Use `%{varname}` or `%varname` in commands:

```
/set target orc
/send kill %{target}
```

**Special forms:**
- `%1` - `%9` - Positional parameters from a macro call or trigger match; `%0` is the macro's own name
- `%*` - All positional parameters
- `%{1-default}` - Positional parameter 1, or `default` if not supplied
- `%-N` / `{-N}` - All but the first N positional parameters, space-joined
- `%L` / `%R` - Text left/right of a trigger match
- `%%` - Literal percent sign
- A run of two or more `%` characters collapses by exactly one per
  substitution pass (see `reference/tf-engine.md`'s "escape-level rule") —
  this is what lets a value survive un-evaluated through one more level of
  macro or `/for` nesting.

**TinyFugue does not expand `%var`/`$[...]`/`$(...)` on a bare top-level
line read from a file** — only inside a macro body, or via `/eval`'s own
substitution pass. Clay matches this: `/set time_format=%H:%M:%S` in a
loaded file stores the format itself. If a script-level probe needs
expansion, wrap it in a macro or prefix it with `/eval`.

**A line you type follows `%sub`**, as in TinyFugue (`/help sub`). With the
default `/sub off`, a command runs exactly as typed — `/echo %{foo}` prints
`%{foo}`. `/sub on` turns `%;` (and `%\`) into line breaks and `%%` into `%`;
`/sub full` processes the whole line like a macro body, so `%{foo}`,
`$[...]` and `$(...)` expand. Key bindings (`/bind`, `/def -b`, `key_<name>`)
always run as macro bodies, and Clay actions keep expanding their commands
as before.

## Typed Lines

Two more TinyFugue input forms work in every interface (console, web, GUI,
Android, and the remote console):

- **`^old^new`** finds the most recent line in your input history that
  contains `old`, replaces the first `old` in it with `new`, shows the result
  and runs it (`% No match.` if no line contains `old`). The new line, not the
  `^old^new` one, goes into history. `^^text` puts `text` in front of the last
  line. Not used in Slack/Discord worlds, where `^^` is ordinary chat, and never
  at a password prompt.
- **Leading slashes**: `/cmd` and `//cmd` both run `/cmd`; three or more
  slashes send the line to the world as text, less two slashes — type
  `///who` to send `/who` to the MUD.

## A note on `#`

In a loaded file, a line starting with `;` or `#` is a comment, as in
TinyFugue — in the first column only: an indented `;` or `#` line is not a
comment, and (like any line that isn't a `/command`) stops the load. Inside a
multi-line `/if`, `/while` or `/for` block written without `\` continuations
(a Clay extension), an indented comment is allowed. `#` is not a second way
to *invoke* a command: only `/` dispatches one; typing `#version` at the
console just sends the literal text `#version` to the current world, it does
not run `/version`.

## Help

`/help <topic>` answers from Clay's own help first; a topic it doesn't cover
comes from TinyFugue's help file when `tf` is installed (`%TFHELP`, else
`tf-help` in `%TFLIBDIR`), as TF would show it - `/help intro`, `/help
special variables`, `/help kecho`. Those topics are case-sensitive, as in TF.

## Differences from TinyFugue (intentional)

Clay is not a byte-for-byte TF clone. Where the two genuinely differ, this
is the list — see `TINYFUGUE-COMPAT.md` for the full rationale and the
per-key/per-command ruling tables this was decided from.

- **Keys**: `^Q` stays spell suggestions (TF: literal-next, which Clay puts
  on `^V`); `^R` stays hot reload; `^L` stays Clay's "redraw keeping only
  server output" (TF's plain repaint is the unbound `refresh_line` action). `Tab` keeps Clay's own more-mode/paging priority
  (`Esc-Tab` does TF-style completion instead). The kill ring (`^Y` to
  yank), the F-keys (help/tags/filter/history-search/highlights/GMCP
  media), `Shift-Up/Down` (cycle all worlds) and `Alt-Up/Down` (resize
  input) are Clay-only additions with no TF equivalent.
- **Commands**: `/recall -D` (also searches the long-term scrollback
  archive), `/world -e` (open the world editor), `/watchdog -w<world>`
  (spam detection), the `/connections` table, `/trigger -d` (delete
  matching triggers), `/repeat -p<priority>`, long-form `/def -a"gag"`,
  `/quote -A` (keep escape sequences) and `/tfhelp` are all Clay extras kept
  alongside TF's own behavior. Clay's remote attach is `/server`; `/connect`
  is TF's.
- **Startup**: `-v` is Clay's "version" (Clay's console is always visual), and
  nothing connects to the first world unless named on the command line
  (`clay mymud`). See "Switching from TinyFugue".
- **No non-visual mode**: `/visual off` is refused; `%visual` is always `on`.
- **Histories**: Clay keeps a world's whole output (and archives it), so
  `/histsize` has nothing to change.
- **Off until asked for**: `%wrapsize` (Clay wraps at the window's edge until a
  script sets it) and mail checking (until a MAIL hook, `@mail` on a changed
  status line, or a mail path set by the user).
- **`/repeat`, `/quote`** with no `-w` stay with the world they started in.
- **`replace()`** now takes TF's own argument order, `replace(old, new,
  str)` — Clay's `/replace` command already used this order; only the
  *function* changed. A release note, since this is a real behavior change
  for anyone who wrote `replace(str, old, new)` expressions.
- **`/bind`** defers substitution to keypress time (`/bind key = cmd` builds
  the same nameless macro `/def -b"key" = cmd` would), matching `/def`'s own
  body semantics — a plain `/bind` used to substitute eagerly, once, at
  bind-registration time.
- **`/quote`, `/repeat`, `/ps`, `/eval`/`/trigger`/`/undefn`/`/not`**
  now match TF's own semantics and option sets exactly (see
  `reference/tf-engine.md` and `/help <command>` for each); `/purge` and
  `/undef` are silent on success like real TF, and a redefinition prints
  TF's own `% Redefined macro X` message unless a REDEF hook gags it.
  `/list`/`/purge`'s own filter semantics are otherwise unchanged from
  Clay's pre-parity behavior.
- **Console-only**: an interactive `/sh`,
  `/suspend`, `/limit`/`/unlimit`/`/relimit` (drive the console's own
  F4 filter popup — a remote client's `/limit` reaches the shared engine
  but nothing drains it until the console next processes a typed command),
  `/xtitle` (sets the *terminal's* title), and the `expand_line` key action
  (no wire path today for a remote client to expand its own input line
  server-side).

## Examples

### Auto-heal Trigger
```
/def -t"Your health: *" -mglob heal_check = /if ({1} < 50) cast heal
```

### Connect Hook
```
/def -hCONNECT auto_look = look
```

### Conditional Response
```
/def -t"* tells you: *" -mglob tell_response = /if ("{1}" =~ "friend") say Hi {1}!
```

### Loop Example
```
/def train_all = /for i 1 5 train str /done
```

\newpage
