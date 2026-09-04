# TF (TinyFugue) Engine Reference

Clay includes a TinyFugue (TF) 5.0 compatibility layer (`src/tf/`). Only `/`
dispatches a command. A line starting with `;`, a bare `#`, or `#` followed
by a space is a **comment** — matching TF's own script convention, not a
second command prefix (`/load`, and loop/`/if` bodies loaded from a file,
skip such lines entirely; typed at the console, an unrecognized `#...` line
is just sent to the current world like any other non-`/` text). See
`docs/markdown/06-tf-commands.md` for the full command list grouped like
`/help commands`, and `TINYFUGUE-COMPAT.md` for the rulings behind every
place Clay's behavior differs from real TF's.

## Command Dispatch

On the console, typed input goes through the TF engine first (a native Clay
command bounces back via `TfCommandResult::ClayCommand`); WS/GUI/web/daemon
clients run Clay's own native command parser first and fall through to TF.
Within the TF engine itself, one name resolves in this order:

1. `/@name` forces the **builtin**, bypassing a same-named macro (TF's own
   builtin-bypass escape hatch — a leading `@` on the command word itself).
2. A **user-defined macro** with this name (TinyFugue precedence: a macro
   shadows a same-named builtin) — except the control-flow keywords
   (`if elseif else endif while for done break`), which a macro can never
   shadow.
3. A **builtin** TF command (the dispatch `match` in `src/tf/parser.rs`).
4. Otherwise, `TfCommandResult::UnknownCommand`.

## Variables

- `/set varname value` - Set global variable (no value: list all)
- `/unset varname` - Remove variable
- `/let varname value` - Set local variable (within macro scope); the value is **not trimmed** — leading/trailing spaces are kept exactly as typed
- `/setenv varname` - Export variable to environment (for `/sh` and `/quote`)
- `/listvar [-mgxsv] [name [value]]` - List variables matching pattern; option flags select macro-local/global/regex/simple/verbose scope
- `:=` (assignment operator, in an expression) updates an existing binding wherever it lives (an enclosing macro's local, or global) rather than always writing the innermost local scope; if no binding exists anywhere, it creates a global

### Variable Substitution
- `%{varname}` - Variable value
- `%varname` - Variable value (simple form, ends at the first non-alphanumeric character)
- `%{varname-default}` / `%varname-default` (unbraced form also accepted) - Value of `varname`, or `default` if unset/empty
- `%1` - `%9` - Positional parameters from a macro call or trigger match; `%0` is the macro's own name
- `%*` - All positional parameters, space-joined
- `%{#}` / `{#}` - Number of positional parameters
- `%-N` / `{-N}` - **All but the first N** positional parameters, space-joined (not "the Nth from the end" — verified against real tf)
- `%L` - Text left of a trigger match
- `%R` (`%-L` in expression-brace form is "right of match", `{PL}`/`{PR}` below are the regex equivalents) - Text right of a trigger match
- `%P0` - `%P9` - Regex capture groups from `regmatch()` or a regexp trigger match (`%P0` = full match); `{P0}`-`{P9}`, `{PL}`, `{PR}` are the same values as expression-brace locals
- `%%` - Literal percent sign; `\%` also means a literal percent sign
- **Escape-level rule**: a run of `N >= 2` consecutive `%` characters collapses to `N - 1` literal `%` characters, and whatever follows is left untouched by *this* substitution pass — it takes one more pass (one per nesting level, e.g. one per level of nested `/for`) to peel off another `%` and get one substitution closer to a live evaluation. This is **not** a simple pairwise `%%`→`%` collapse repeated to exhaustion: a 3-run of `%` becomes 2 literal `%` characters, not "collapse one pair, evaluate the rest." This is how a multiply-nested macro (or `/for`) protects an inner substitution from firing before its own iteration is ready — see tf-help `/for`'s own `%%{...}` example and `color.tf`'s triple-nested `%%%{red}`.
- **`/eval` does one *extra* substitution pass** beyond the one every command's own arguments already get — see "Expressions" below.

### Special Variables
- `%{world_name}`, `%{world_host}`, `%{world_port}`, `%{world_character}` - Current world info
- `%{pid}`, `%{time}`, `%{version}`, `%{nworlds}`, `%{nactive}`
- `%{kbnum}` - The pending numeric-prefix magnitude (`Esc-0`..`Esc-9`/`Esc--`); consumed and cleared by movement/scroll/delete actions and `/dokey`
- `%{insert}` - `1` when insert mode is on, `0` when overwrite mode is on (`Insert`/`Esc-v`)
- `%{TFLIBDIR}` - The resolved TinyFugue library directory (see "Loading Library Files" below)
- `%{TFPATH}` - Colon-separated search path for `/load`/`/require` (checked before `%{TFLIBDIR}`)
- `%{maxpri}` - `2147483647`, seeded at engine start (kbfunc.tf's own `-ip%maxpri` idiom needs this without loading stdlib.tf first)

## Output

- `/echo [-a<attrs>] [-p] [-o|-e|-A|-r] [-w[<world>]] [--] message` - Display a local message. `-a<attrs>` wraps it in ANSI codes; `-w` (bare = current world) echoes into another world's window; `-r` suppresses `@{...}` attribute-sequence interpretation.
  - ANSI attributes inline: `@{B}` bold, `@{U}` underline, `@{I}` inverse, `@{D}` dim, `@{F}` flash, `@{n}` normal/reset
  - Colors: `@{Crgb}` foreground (r,g,b = 0-5), `@{BCrgb}` background, `@{Cname}` named colors
- `/send [-W] [-T<type>] [-w[world]] [-n] [-h] text` - Send text to a world, bypassing macro/alias expansion. `-W` = every connected world; `-T<type>` = every connected world of that type; `-n` = no end-of-line marker; `-h` fires the SEND hook first (off by default for `/send`)
- `/beep [on|off]` - Terminal bell (bare: ring it now; `on`/`off` toggles the setting)
- `/quote [options] [prefix]source[suffix]` - Generate and send/echo/execute text from a file, a command's output, a shell command, or literal text
  - Sources: `'"file"'` (file), `` `"command" `` (Clay/TF command's own output, finding 14), `!"command"` (shell output), or literal text
  - Options: `-dsend` (default) / `-decho` (display locally) / `-dexec` (run each line back through the engine); `-w<world>`
- `/substitute [-p] text` - Run `text` through the current world's SUBSTITUTE hook processing (`-p`: preview only, doesn't display)
- `/hilite [pattern [= response]]`, `/nohilite [pattern]`, `/partial regexp` - Shortcuts for a highlighting trigger (`/help hilite`/`nohilite`/`partial` for the exact `/def` equivalents)
- `/gag <pattern>` / `/ungag <pattern>` - Shortcuts for a gag trigger

## Expressions

- `/expr expression` - Evaluate and display result
- `/test expression` - Evaluate and return its value, setting `%?` (unlike `/expr`, doesn't auto-display)
- `/eval [-s<level>] text` - One more substitution pass on `text` (`%vars`, `$[...]`, `$(...)`, and the `%;` separator), then execute the result: a `/`-command runs through the normal macro-or-builtin lookup, anything else is sent to the world. `-s0` skips the extra substitution pass and dispatches `text` exactly as given.
- `/not [-s<level>] command` - Identical substitution/dispatch to `/eval`, but sets `%?` to the **logical negation** of whatever the command left in `%?`
- Operators: `+ - * / %` (arithmetic, wrapping on overflow), `== != < > <= >=` (comparison), `& | !` (logical), `=~ !~` (regex), `=/ !/` (glob), `?:` (ternary), `,` (comma - evaluates both sides, yields the right)
- `$[expression]` and `$(command)` are real expression-primitive tokens (not textual pre-substitution): each is resolved **lazily**, so a ternary's untaken branch never runs its own side effects

### String Functions
`strlen()` `substr(s, start[, len])` `strcat(...)` `strstr()` `strchr()` `strrchr()` `strcmp()` `strncmp()` `strrep(s, n)` `tolower()` `toupper()` `escape(meta, s)` `replace(old, new, s[, count])` — **TF's argument order** (Clay's own `/replace` command already used this order; the `replace()` *function* changed to match, see `TINYFUGUE-COMPAT.md`) `tr(domain, range, s)` `ascii()` `char()` `sprintf(fmt, ...)` `pad(s, w, ...)` `strip_attr()` `encode_attr()` `decode_attr(s[, attrs[, f]])` `encode_ansi()` `decode_ansi()` `strcmpattr()`

### Math Functions
`abs()` `min()` `max()` `mod(i, j)` `trunc()` `rand([max])` / `rand(min, max)` `sin/cos/tan/asin/acos/atan()` `exp()` `pow()` `sqrt()` `log()` `log10()`

### Regex
`regmatch(pattern, string)` - Match and populate `%P0`-`%P9`/`{P0}`-`{P9}`/`{PL}`/`{PR}` (both the text-substitution array and the expression-brace LOCAL VARIABLES a trigger match itself sets)

### World Functions
`fg_world()` `world_info(field[, world])` `nactive()` `nworlds()` `is_connected([world])` `idle([world])` `sidle([world])` `addworld(name, type, host, port, char, pass, file, flags, srchost)`

### Info Functions
`columns()` `lines()` `moresize()` `getpid()` `systype()` `filename()` `ftime([format])` (one-argument form uses the current time) `nmail()` `features()`

### Macro/Command Functions
`ismacro(name)` `getopts(optstring, varname)` `echo(text[, attrs])` `send(text[, world])` `substitute(text[, attrs])` `keycode(str)`, plus function forms of `prompt`, `eval`, `def`, `test`

### Keyboard Buffer Functions
`kbhead()` `kbtail()` `kbpoint()` `kblen()` `kbgoto(pos)` `kbdel(n)` `kbmatch()` `kbword()` `kbwordleft()` `kbwordright()` `input(text)`

### File I/O Functions
`tfopen(path, mode)` `tfclose(handle)` `tfread(handle, var)` `tfwrite(handle, text)` `tfflush(handle)` `tfeof(handle)`

## Control Flow

- `/if (expr) command` / `/if (expr) ... /elseif (expr) ... /else ... /endif` - Conditional (parenthesized-expression form)
- `/if /command%; /then list [/elseif /command%; /then list]... [/else list] /endif` - Conditional (command form): the command's own return status (`%?`, nonzero = true) is the condition; a leading `/!` negates it
- `/while (expr) ... /done` / `/while /command%; /do list /done` - While loop (same two forms; the command form re-runs its command fresh before every iteration)
- `/for var min max command` - TinyFugue's own form: `var` takes every integer from `min` to `max` inclusive (counting up only), substituted fresh each iteration
- `/for var start end [step] ... /done` - Clay extension: an explicit numeric step (default 1, or -1 when `end < start`), body collected up to `/done`
- `/break [n]` - Unconditionally exit the nearest enclosing loop (or `n` levels)
- `/result [expression]` - Like `/return`, but when the macro was called as a *command* (not a function), also echoes the value — so the same macro works usefully either way
- `/return [expression]` - Stop the macro, set `%?`; never echoes

## Macros (Triggers)

- `/def [options] [name] = body` - Define a macro. `name` is optional if `-t`, `-b`, `-B`, or `-h` is given — such a macro is addressed only by its number (shown by `/list`, or `%?` right after `/def`).
  - `-t"pattern"` - Trigger pattern; `-mtype` - match type `simple`/`glob`(default)/`regexp`
  - `-p priority` - Execution priority (higher runs first); `-F` - fall-through to other triggers; `-1` - one-shot; `-n count` - fire only N times
  - `-ag`/`-ah`/`-ab`/`-au` - gag/highlight/bold/underline (long forms `-a"gag"` etc. also accepted); `-E"expr"` - conditional; `-c chance` - probability
  - `-w world` / `-T type` - restrict to a world (by name, or by type)
  - `-hEVENT` - hook event, matches every occurrence; `-h"EVENT pattern"` - hook with an argument pattern (matched like `-t`)
  - `-b"key"` - key binding (identical to `/bind key = body` — both build the same nameless macro, deferring substitution to keypress, not bind time)
  - `-B<name>` - bind by TF's named-key vocabulary instead of a raw sequence (`-Bf5`, `-Bctrl_left`) — deprecated upstream but accepted
  - `-i`, `-I` - invisible (hidden from `/list`/`/save`/`/purge` unless forced); `-q` - quiet (doesn't count toward the BACKGROUND hook or `/trigger`'s return value; a SEND hook doesn't suppress the input); `-f` - same as `-a`, kept for compatibility
  - Bundled short options are accepted (`-iFp9999`, `-ip2`)
  - Redefining a macro prints TF's default `% Redefined macro X` message unless a REDEF hook gags it
- `/undef name...` - Remove macro(s) by name; silent on success
- `/undefn number...` - Remove macro(s) by sequence number (see `/list`, or `%?` right after `/def`); silent on success
- `/undeft pattern` - Remove macros matching a trigger pattern
- `/list [-i -t -b -h -m<style> -s<sort> -a -w<world>] [pattern]` / `/purge [same options] [pattern]` - List / remove macros matching a filter; `/purge` is silent on success
- `/ismacro name` - True iff a macro (or builtin) of this name exists

## Hooks

Hooks fire macros when something happens inside Clay, the same way triggers
fire on MUD output. Register with `/def -hEVENT name = body` (every
occurrence) or `/def -h"EVENT pattern" name = body` (pattern matched like
`-t`), or the equivalent `/hook EVENT[ pattern] [= body]`. Manage with
`/hook` (list), `/unhook EVENT [pattern]`, or fire one manually for testing
with `/trigger -hEVENT text`.

All 32 real TF events are recognized: `ACTIVITY BAMF BGTEXT BGTRIG CONFAIL
CONFLICT CONNECT DISCONNECT ICONFAIL KILL LOAD LOADFAIL LOG LOGIN MAIL MORE
NOMACRO PENDING PREACTIVITY PROCESS PROMPT PROXY REDEF RESIZE SEND SHADOW
SHELL SIGHUP SIGTERM SIGUSR1 SIGUSR2 WORLD` — plus two Clay-only extras,
`GMCP` and `MSDP`, for those protocols. `SEND` is special: a non-quiet (no
`-q`) matching SEND hook *replaces* the text about to be sent with its own
body instead of sending it (this is how `/alias` and speedwalking work); a
quiet SEND hook runs alongside the text without suppressing it. `LOADFAIL`
fires when `/load`/`/require` can't find or open a file — a gagged LOADFAIL
hook suppresses the default error message (stdlib.tf's own guard around an
optional, legitimately-missing `local.tf` relies on exactly this).

## Key Bindings

- `/bind [sequence [= command]]` - `/bind seq = cmd` is exactly `/def -b"seq" = cmd` (substitution deferred to keypress, not bind time); bare `/bind` lists everything, `/bind seq` shows one binding
- `/unbind sequence` - Remove the binding
- `/dokey NAME` - Invoke one of TF's 35 built-in editing primitives directly (`LEFT`, `RIGHT`, `RECALLB`, `PGUP`, `REDRAW`, ...) — a single, non-kbnum-multiplied step
- `dokey_<name>` (e.g. `/dokey_home`, `/dokey_left`) - Native commands mirroring kbfunc.tf's own wrapper macros: unlike bare `/dokey`, the movement-related ones honor a pending numeric prefix (`%kbnum`)
- `/def key_<name> = ...` - TF's two-level named-key layer: a physical key (`F5`, `Ctrl-Left`, `Esc-Left`) is named `f5`/`ctrl_left`/`esc_left`; redefining `key_<name>` changes what the key does independent of which raw sequence the terminal actually sends. `key_meta_<x>` falls back to `key_esc_<x>` when undefined. Dispatch order for a pressed key: `/bind` match, then a `key_<name>` macro, then Clay's built-in action table (`keybindings.dat`) — see `docs/markdown/07-keyboard-shortcuts.md` and `reference/commands.md`.
- Key names: `F1`-`F20`, `^A`-`^Z` (Ctrl), `Esc-x`/`Alt-x`/`Meta-x`/`@x` (equivalent — case preserved), `Ctrl-Up`/`Shift-Tab`/`Alt-Down` (real terminal modifiers), `PgUp`/`PgDn`/`Home`/`End`/`Insert`/`Delete`/`Tab`, and chords (`^X^R`, `Esc-Left`) — see `/help bind` for TF's raw escape-byte spellings.

## Loading Library Files

- `/load [-q] filename` - Load and execute a TF script file. Comments: a line starting with `;`, a bare `#`, or `#` followed by a space. Line continuation: trailing `\` (use `%\` for a literal trailing backslash).
- `/require [-q] filename` - Like `/load`, but does nothing if the file already registered a `/loaded` token
- `/loaded token` - Mark a file loaded (for `/require`); should be the file's first command
- File search order for a bare filename (no `/`): the current directory (`/lcd`/actual cwd), then each directory in `%{TFPATH}` (colon-separated), then `%{TFLIBDIR}`
- `%{TFLIBDIR}` defaults to `$TFLIBDIR` if that names a real directory, else `/usr/share/tf5/tf-lib` if it exists (the path the `tf5` distro package installs to), else unset. **Nothing GPL-licensed ships with Clay** — the real TF library is only ever *referenced* on a machine that already has it installed; a script that `/require`s a library file simply fails to find it (or the whole feature is skipped) on a machine without one.
- **TinyFugue does not expand `%var`, `$[...]`, or `$(...)` on a top-level line read from a file** — only inside a macro body, or when a command explicitly asks for it (`/eval`). A bare top-level `/echo len=$[strlen("abc")]` in a loaded file prints the *literal*, unexpanded text under real TF. Clay matches this: substitution only happens inside macro bodies and via `/eval`'s own pass, never unconditionally on a bare top-level command's arguments read from a file. See `tests/tf/README.md`'s "C.12 rule" for how the test suite writes probes that need expansion.
- `/exit [n]` - Abort loading the current file early (and `n` enclosing `/load`'s, default 1)
- `/save [-a] [filters...] filename` - Save macros to a file

## World Commands

- `/fg [-n -s -q -l -c<N> -< ->] [world]` - Switch to (or show) the foreground world; `-<`/`->` cycle connected worlds (`SOCKETB`/`SOCKETF`); `-n` backgrounds (TF's `/bg`)
- `/addworld [-pxe] [-T<type>] [-s<srchost>] name [char pass] host port [file]` / `/addworld name` / `/addworld DEFAULT [char pass [file]]` - Create/update a world, or set fallback character/password/file for worlds missing their own
- `/dc` / `/disconnect [<world>|-ALL]` - Disconnect one world, or every connected world
- `/listworlds [-c -u -s -m -S<field> -T<type>]`, `/listsockets`/`/connections`/`/l [-s -n -m -S<field>]` - World/connection tables
- `/watchdog [-w<world>] pattern` / `/watchname pattern` - Clay's own spam-detection extras

## Miscellaneous

- `/time [format]` - Display the current time (`ftime()`-style `format`, default `%{time_format}`); `/time /command` (Clay extra) times a command instead of printing the clock
- `/runtime command` - Run `command`, then print `real=<secs> cpu=<secs>`
- `/trigger [-ln] [-g] [-w[world]] [-h[event]] [-d] text` - Run `text` through the real trigger (or hook, with `-h`) matcher as if it arrived from a world. `-n`/`-l` list matches without firing; `-d` deletes matching triggers (Clay extra)
- `/version` - Show TF compatibility version; `/tfhelp [topic]` - TF text help (vs `/help`'s Clay popup)
- `/ps [-srq] [-w[world]] [pid]` - List background `/repeat`/`/quote` processes; `/kill pid...` - kill one or more
- `/repeat [-w[world]] {-time|-S|-P} count command` - Schedule a repeated command (`-p priority` sets ordering; higher runs first)
- `/sh [-q] [command]` - Execute a shell command (bare `/sh` opens an interactive shell where supported)
- `/recall [-D] [-w<world>] [-ligv] [-t[format]] [-a<attrs>] [-m<style>] [-A/-B/-C<n>] [#]range [pattern]` - Search output/input history; `-D` (Clay extra) also searches the long-term scrollback archive
- `/histsize [-w<world>] n` - Set history size; `/localecho` - toggle local echo; `/sub`/`/substitute` - run text through SUBSTITUTE hook processing
- `/input text` - Insert text into the input buffer at the cursor; `/grab [world]` - grab the world's last output line into the input buffer
- `/more [-w<world>] [on|off]` / `/wrap [-w<world>] n` - Console-only: more-mode pause and word-wrap width
- `/limit [-v] [-a] [-m<style>] [pattern]` / `/unlimit` / `/relimit` - Console-only: open/close/reapply the F4 filter popup as a text filter (no equivalent live-updating filter view exists on web/GUI clients today — a remote client's own `/limit` reaches the shared TF engine but nothing drains it until the console next processes a typed command)
- `/restrict [none|shell|file|world]` - Raise (never lower) the sandboxing level: `shell` disables `/sh`/`/sys`/`` /quote ! ``; `file` (implies `shell`) also disables `/load`/`/require`/`/save`/`/lcd`/`/cd`/`/log`/`` /quote ' ``; `world` (implies `file`) also disables `/addworld` and connecting to an arbitrary host/port
- `/xtitle text` - Console-only: set the terminal title
- `/features [name]` - Report which optional Clay/TF features are compiled in

## Console-Only Commands

A few commands only make sense (or are currently only wired up) on the
interactive console, not the web/GUI/remote clients: `/limit`/`/unlimit`/
`/relimit` (drive the console's own F4 filter popup), `/xtitle` (sets the
*terminal's* title), and the `expand_line` key action (`Esc-^E` — no wire
path exists today for a remote client to ask the server to substitute its
own input line in place).

## Examples

**Auto-heal trigger:**
```
/def -t"Your health: *" -mglob heal_check = /if ({1} < 50) cast heal
```

**Connect hook:**
```
/def -hCONNECT auto_look = look
```
