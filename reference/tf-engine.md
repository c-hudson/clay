# TF (TinyFugue) Engine Reference

Clay includes a TinyFugue compatibility layer. TF commands work with both `/` and `#` prefixes (unified command system).

## Unified Command System
- All TF commands work with `/` prefix: `/set`, `/echo`, `/def`, etc.
- The `#` prefix still works for backward compatibility: `#set`, `#echo`, `#def`, etc.
- Conflicting commands use `/tf` prefix for TF version: `/tfhelp` (TF text help) vs `/help` (Clay popup), `/tfgag` (TF gag pattern) vs `/gag` (Clay action gag)

## Variables

- `/set varname value` (or `#set`) - Set global variable
- `/unset varname` - Remove variable
- `/let varname value` - Set local variable (within macro scope)
- `/setenv varname` - Export variable to environment
- `/listvar [pattern]` - List variables matching pattern

### Variable Substitution
- `%{varname}` - Variable value
- `%varname` - Variable value (simple form)
- `%1` - `%9` - Positional parameters from trigger match
- `%*` - All positional parameters
- `%L` - Text left of match
- `%R` - Text right of match
- `%P0` - `%P9` - Regex capture groups from `regmatch()` or trigger match (%P0 = full match)
- `%%` - Literal percent sign
- `\%` - Literal percent sign (backslash escape)

### Capture Groups in Expressions
- Trigger capture groups are available in both text substitution (`%P1`) and expression context (`{P1}`)
- `{P0}` - Full match, `{P1}`-`{P9}` - Capture groups, `{PL}` - Left of match, `{PR}` - Right of match
- These are set as local variables in macro scope so the expression evaluator can resolve them

### Special Variables
- `%{world_name}` - Current world name
- `%{world_host}` - Current world hostname
- `%{world_port}` - Current world port
- `%{world_character}` - Current world username
- `%{pid}` - Process ID
- `%{time}` - Unix timestamp
- `%{version}` - TF compatibility version string
- `%{nworlds}` - Total number of worlds
- `%{nactive}` - Number of connected worlds

## Output

- `/echo [-w world] message` - Display local message (supports `%{var}` substitution and ANSI attributes)
  - ANSI attributes: `@{B}` bold, `@{U}` underline, `@{I}` inverse, `@{D}` dim, `@{F}` flash, `@{n}` normal/reset
  - Colors: `@{Crgb}` foreground (r,g,b = 0-5), `@{BCrgb}` background, `@{Cname}` named colors (red, green, blue, cyan, magenta, yellow, white, black)
- `/send [-w world] text` - Send text to MUD
- `/beep` - Terminal bell
- `/quote [options] [prefix] source [suffix]` - Generate and send text from file, command, or literal
  - Sources: `'"file"` (read from file), `` `"command" `` (read internal command output), `!"command"` (read shell output), or literal text
  - Options: `-dsend` (default), `-decho` (display locally), `-dexec` (execute as TF), `-wworld`
  - Example: `/quote say '"/tmp/lines.txt"` sends "say <line>" for each line in file
  - Example: `` /quote think `"/version" `` sends "think Clay v1.0..." to MUD
  - Example: `/quote !"ls -la"` sends output of shell ls command to MUD

## Expressions

- `/expr expression` - Evaluate and display result
- `/test expression` - Evaluate as boolean (returns 0 or 1)
- `/eval expression` - Evaluate and execute result as command
- Operators: `+ - * / %` (arithmetic), `== != < > <= >=` (comparison), `& | !` (logical), `=~ !~` (regex), `=/ !/` (glob), `?:` (ternary)

### String Functions
`strlen()`, `substr()`, `strcat()`, `tolower()`, `toupper()`, `strstr()`, `replace()`, `sprintf()`, `strcmp()`, `strncmp()`, `strchr()`, `strrchr()`, `strrep()`, `pad()`, `ascii()`, `char()`

### Math Functions
`rand()`, `time()`, `abs()`, `min()`, `max()`, `mod()`, `trunc()`, `sin()`, `cos()`, `tan()`, `asin()`, `acos()`, `atan()`, `exp()`, `pow()`, `sqrt()`, `log()`, `log10()`

### Regex
`regmatch(pattern, string)` - Match and populate %P0-%P9 capture groups

### World Functions
`fg_world()`, `world_info(field[, world])`, `nactive()`, `nworlds()`, `is_connected([world])`, `idle([world])`, `sidle([world])`, `addworld()`

### Info Functions
`columns()`, `lines()`, `moresize()`, `getpid()`, `systype()`, `filename()`, `ftime()`, `nmail()`

### Macro Functions
`ismacro(name)`, `getopts(optstring, varname)`

### Command Functions
`echo(text[, attrs])`, `send(text[, world])`, `substitute(text[, attrs])`, `keycode(str)`

### Keyboard Buffer Functions
`kbhead()`, `kbtail()`, `kbpoint()`, `kblen()`, `kbgoto(pos)`, `kbdel(n)`, `kbmatch()`, `kbword()`, `kbwordleft()`, `kbwordright()`, `input(text)`

### File I/O Functions
`tfopen(path, mode)`, `tfclose(handle)`, `tfread(handle, var)`, `tfwrite(handle, text)`, `tfflush(handle)`, `tfeof(handle)`

## Control Flow

- `/if (expr) command` - Single-line conditional
- `/if (expr) ... /elseif (expr) ... /else ... /endif` - Multi-line conditional
- `/while (expr) ... /done` - While loop
- `/for var start end [step] ... /done` - For loop
- `/break` - Exit loop early

## Macros (Triggers)

- `/def [options] name [= body]` - Define macro (body is optional for attribute-only macros)
  - `-t"pattern"` - Trigger pattern (fires when MUD output matches)
  - `-mtype` - Match type: `simple`, `glob` (default), `regexp`
  - `-p priority` - Execution priority (higher = first)
  - `-F` - Fall-through (continue checking other triggers)
  - `-1` - One-shot (delete after firing once)
  - `-n count` - Fire only N times
  - `-a` - Attributes: supports both single-letter TF codes (`g`=gag, `h`=hilite, `B`=bold, `u`=underline, `r`=reverse, `b`=bell) and long-form names (`"gag"`, `"bold"`, etc.)
  - `-ag` - Gag (suppress) matched line
  - `-ah` - Highlight matched line
  - `-ab` - Bold
  - `-au` - Underline
  - `-E"expr"` - Conditional (only fire if expression is true)
  - `-c chance` - Probability (0.0-1.0)
  - `-w world` - Restrict to specific world
  - `-h event` - Hook event (CONNECT, DISCONNECT, etc.)
  - `-b"key"` - Key binding
- `/undef name` - Remove macro
- `/undefn pattern` - Remove macros matching name pattern
- `/undeft pattern` - Remove macros matching trigger pattern
- `/list [pattern]` - List macros
- `/purge [pattern]` - Remove all macros (or matching pattern)

## Hooks

- `/def -hCONNECT name = command` - Fire on connect
- `/def -hDISCONNECT name = command` - Fire on disconnect
- Events: `CONNECT`, `DISCONNECT`, `LOGIN`, `PROMPT`, `SEND`, `ACTIVITY`, `WORLD`, `RESIZE`, `LOAD`, `REDEF`, `BACKGROUND`

## Key Bindings

- `/bind key = command` - Bind key to command
- `/unbind key` - Remove binding
- Key names: `F1`-`F12`, `^A`-`^Z` (Ctrl), `@a`-`@z` (Alt), `PgUp`, `PgDn`, `Home`, `End`, `Insert`, `Delete`

## File Operations

- `/load filename` - Load TF script file
- `/save filename` - Save macros to file
- `/lcd path` - Change local directory

## World Commands

- `/fg [world]` - Switch to specified world (or show current)
- `/addworld [-Lq] name host port` - Create a new world

## Input Commands

- `/input text` - Insert text into input buffer at cursor
- `/grab [world]` - Grab last line from world's output into input buffer
- `/trigger [pattern]` - Manually trigger macros matching pattern

## Miscellaneous

- `/time` - Display current time
- `/version` - Show TF compatibility version
- `/tfhelp [topic]` - Show TF text help (vs `/help` for Clay popup)
- `/ps` - List background processes
- `/kill id` - Kill background process
- `/repeat [-p priority] time count command` - Schedule repeated command (-p sets priority, higher = runs first)
- `/sh command` - Execute shell command
- `/recall [pattern]` - Search output history

## Examples

**Auto-heal trigger:**
```
/def -t"Your health: *" -mglob heal_check = /if ({1} < 50) cast heal
```

**Connect hook:**
```
/def -hCONNECT auto_look = look
```
