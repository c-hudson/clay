# TinyFugue Commands

Clay includes a TinyFugue compatibility layer using the `#` prefix. This allows TF veterans to use familiar commands and scripting patterns.

## Variables

### #set / #unset
Set or remove global variables.

```
#set varname value      # Set variable
#unset varname          # Remove variable
```

### #let
Set a local variable (within macro scope).

```
#let temp_value 100
```

### #setenv
Export a variable to the environment.

```
#setenv MY_VAR
```

### #listvar
List variables matching a pattern.

```
#listvar              # List all variables
#listvar hp*          # List variables starting with "hp"
```

## Output Commands

### #echo
Display a local message (not sent to MUD). Supports variable substitution.

```
#echo Hello, world!
#echo Your HP is %{hp}
```

### #send
Send text to the MUD.

```
#send look
#send -w MyMUD say hello    # Send to specific world
```

### #beep
Play terminal bell.

```
#beep
```

### #quote
Send text without variable substitution.

```
#quote This %{var} stays literal
```

## Expressions

### #expr
Evaluate and display an expression result.

```
#expr 5 + 3           # Displays: 8
#expr strlen("hello") # Displays: 5
```

### #test
Evaluate expression as boolean (returns 0 or 1).

```
#test 5 > 3           # Returns: 1
#test hp < 50         # Returns: 1 or 0
```

### #eval
Evaluate expression and execute result as command.

```
#set cmd "look"
#eval cmd             # Executes: look
```

### Expression Operators

| Category | Operators |
|----------|-----------|
| Arithmetic | `+` `-` `*` `/` `%` |
| Comparison | `==` `!=` `<` `>` `<=` `>=` |
| Logical | `&` `\|` `!` |
| Regex | `=~` `!~` |
| Ternary | `? :` |

### Built-in Functions

| Function | Description |
|----------|-------------|
| `strlen(s)` | String length |
| `substr(s, start, len)` | Substring |
| `strcat(s1, s2)` | Concatenate strings |
| `tolower(s)` | Lowercase |
| `toupper(s)` | Uppercase |
| `rand()` | Random number |
| `time()` | Current time |
| `abs(n)` | Absolute value |
| `min(a, b)` | Minimum |
| `max(a, b)` | Maximum |

## Control Flow

### #if / #elseif / #else / #endif
Conditional execution.

```
# Single-line
#if (hp < 50) cast heal

# Multi-line
#if (hp < 25)
  cast 'cure critical'
#elseif (hp < 50)
  cast heal
#else
  #echo HP is fine
#endif
```

### #while / #done
While loop.

```
#while (count < 10)
  #echo Count: %{count}
  #set count %{count} + 1
#done
```

### #for / #done
For loop.

```
#for i 1 10
  #echo Number: %{i}
#done

#for i 1 10 2        # Step by 2
  #echo Odd: %{i}
#done
```

### #break
Exit loop early.

```
#while (1)
  #if (done) #break
#done
```

## Macros (Triggers)

### #def
Define a macro with optional trigger pattern.

```
#def name = command body

# With trigger pattern
#def -t"pattern" name = command

# With options
#def -t"* tells you: *" -mglob reply_tell = say Thanks, $1!
```

**Options:**

| Option | Description |
|--------|-------------|
| `-t"pattern"` | Trigger pattern |
| `-mtype` | Match type: `simple`, `glob`, `regexp` |
| `-p priority` | Execution priority (higher = first) |
| `-F` | Fall-through (continue checking other triggers) |
| `-1` | One-shot (delete after firing) |
| `-n count` | Fire only N times |
| `-ag` | Gag (suppress) matched line |
| `-ah` | Highlight matched line |
| `-ab` | Bold |
| `-au` | Underline |
| `-E"expr"` | Conditional expression |
| `-c chance` | Probability (0.0-1.0) |
| `-w world` | Restrict to specific world |
| `-h event` | Hook event |
| `-b"key"` | Key binding |

### #undef / #undefn / #undeft
Remove macros.

```
#undef name           # Remove by name
#undefn pattern       # Remove matching name pattern
#undeft pattern       # Remove matching trigger pattern
```

### #list
List defined macros.

```
#list                 # List all
#list heal*           # List matching pattern
```

### #purge
Remove all macros (or matching pattern).

```
#purge                # Remove all
#purge temp_*         # Remove matching pattern
```

## Hooks

Define macros that fire on events:

```
#def -hCONNECT auto_look = look
#def -hDISCONNECT goodbye = #echo Disconnected!
```

**Available events:**
- `CONNECT` - When connected to a world
- `DISCONNECT` - When disconnected
- `LOGIN` - After auto-login completes
- `PROMPT` - When prompt is received
- `SEND` - Before command is sent
- `ACTIVITY` - When activity occurs in background world
- `WORLD` - When world changes
- `RESIZE` - When terminal resizes
- `LOAD` - When script is loaded
- `REDEF` - When macro is redefined
- `BACKGROUND` - When world goes to background

## Key Bindings

### #bind / #unbind
Bind keys to commands.

```
#bind F5 = cast heal
#unbind F5
```

**Key names:**
- `F1` - `F12`
- `^A` - `^Z` (Ctrl+letter)
- `@a` - `@z` (Alt+letter)
- `PgUp`, `PgDn`, `Home`, `End`, `Insert`, `Delete`

## File Operations

### #load
Load a TF script file.

```
#load scripts/my_triggers.tf
```

### #save
Save macros to a file.

```
#save macros_backup.tf
```

### #lcd
Change local directory.

```
#lcd /home/user/mud
```

## Variable Substitution

Use `%{varname}` or `%varname` in commands:

```
#set target orc
#send kill %{target}
```

**Special variables:**
- `%1` - `%9` - Positional parameters from trigger match
- `%*` - All positional parameters
- `%L` - Text left of match
- `%R` - Text right of match
- `%%` - Literal percent sign

## Examples

### Auto-heal Trigger
```
#def -t"Your health: *" -mglob heal_check = \
  #if ({1} < 50) cast heal
```

### Connect Hook
```
#def -hCONNECT auto_look = look
```

### Conditional Response
```
#def -t"* tells you: *" -mglob tell_response = \
  #if ("{1}" =~ "friend") say Hi {1}!
```

### Loop Example
```
#def train_all = \
  #for i 1 5; \
    train str; \
  #done
```

\newpage

