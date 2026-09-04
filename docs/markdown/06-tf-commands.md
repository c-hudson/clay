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
Display a local message (not sent to the MUD). Variable substitution runs
before `/echo` ever sees the text.

```
/echo Hello, world!
/echo Your HP is %{hp}
```

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

### /quote
Generate and dispatch text from a file, another command's output, a shell
command, or literal text.

```
/quote say '"/tmp/lines.txt"       Send "say <line>" for each line in the file
/quote think `"/version"           Send the output of /version to the MUD
/quote !"ls -la"                   Send the output of a shell command
```

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
behavior.

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
Load a TF script file. Comments: a line starting with `;`, a bare `#`, or
`#` followed by a space (see "A note on `#`" below).

```
/load scripts/my_triggers.tf
```

### /require
Like `/load`, but a file that's already registered a `/loaded` token isn't
read again — and, unlike `/load`, a *bare filename* (no `/` in it) is also
searched for along `%{TFPATH}` and then `%{TFLIBDIR}`.

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
`%{TFPATH}` and then `%{TFLIBDIR}`, and `%{TFLIBDIR}` defaults to
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
substitution pass. Clay matches this. If a script-level probe needs
expansion, wrap it in a macro or prefix it with `/eval`.

## A note on `#`

Real TinyFugue's comment character is `;`; Clay additionally treats a bare
`#`, or `#` followed by a space, as a comment too (in a loaded file, and in
loop/`/if` bodies) — this is a superset of TF's own convention, not a
second way to *invoke* a command. Only `/` dispatches a command; typing
`#version` at the console just sends the literal text `#version` to the
current world, it does not run `/version`.

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
  `#`/`# ` comments in `/load`, and `/tfhelp` are all Clay extras kept
  alongside TF's own behavior.
- **`replace()`** now takes TF's own argument order, `replace(old, new,
  str)` — Clay's `/replace` command already used this order; only the
  *function* changed. A release note, since this is a real behavior change
  for anyone who wrote `replace(str, old, new)` expressions.
- **`/bind`** defers substitution to keypress time (`/bind key = cmd` builds
  the same nameless macro `/def -b"key" = cmd` would), matching `/def`'s own
  body semantics — a plain `/bind` used to substitute eagerly, once, at
  bind-registration time.
- **`/histsize`, `/quote`, `/repeat`, `/eval`/`/trigger`/`/undefn`/`/not`**
  now match TF's own semantics and option sets exactly (see
  `reference/tf-engine.md` and `/help <command>` for each); `/purge` and
  `/undef` are silent on success like real TF, and a redefinition prints
  TF's own `% Redefined macro X` message unless a REDEF hook gags it.
  `/list`/`/purge`'s own filter semantics are otherwise unchanged from
  Clay's pre-parity behavior.
- **Console-only**: `/limit`/`/unlimit`/`/relimit` (drive the console's own
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
