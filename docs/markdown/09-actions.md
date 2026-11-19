# Actions (Triggers)

Actions are automated triggers that match incoming MUD output and execute commands. They're essential for automating repetitive tasks like healing, responding to tells, or filtering spam.

## Opening the Actions Editor

```
/actions                 # Every action
/actions -w MyMUD        # Only MyMUD's actions (-wMyMUD works too)
/actions heal_self       # Edit the action named heal_self
/actions heal_*          # * and ? are wildcards
```

- `-w` lists the actions whose World field holds what you give, whole or in part
  (`-w zmc` finds `ZMC` and `zmc2`); case is ignored.
- A name opens that action's editor straight away, case ignored. Closing the editor
  goes back to the output, since no list was opened.
- In a name, `*` stands for any run of characters and `?` for one. One match opens its
  editor; several are shown in the Actions list, which then lists only them (the note
  under its filter box says so). No match prints a line saying so.
- `-w` and a name together narrow both ways: `/actions -w zmc *_idle`.

![Actions List](images/tui/actions-list.png)

## Action List Controls

| Keys | Action |
|------|--------|
| `Up/Down` | Navigate list |
| `Space` | Toggle enable/disable |
| `Enter` | Edit selected action |
| `A` | Add new action |
| `E` | Edit selected action |
| `D` | Delete selected action |
| `/` or `F` | Focus filter box |
| `Esc` | Close popup |

## Creating an Action

Press `A` to add a new action:

![Action Editor](images/tui/actions-editor.png)

### Action Fields

| Field | Description |
|-------|-------------|
| Name | Action name (also used for manual invocation) |
| World | Restrict to specific world (empty = all worlds) |
| Match Type | Regexp or Wildcard |
| Pattern | Trigger pattern (empty = manual-only) |
| Command | Commands to execute (multiline, semicolon-separated) |
| Enabled | Whether the action is active |

## Match Types

### Regexp (Regular Expression)

Full regex syntax for precise pattern matching:

| Pattern | Matches |
|---------|---------|
| `^You say` | Lines starting with "You say" |
| `tells you:` | Lines containing "tells you:" |
| `HP: (\d+)` | Captures the number after "HP: " |
| `^\[.*\]` | Lines starting with bracketed text |

### Wildcard (Glob-style)

Simple pattern matching where:
- `*` matches any sequence of characters
- `?` matches any single character
- `\*` and `\?` match literal asterisk/question mark

| Pattern | Matches |
|---------|---------|
| `*tells you*` | Any line containing "tells you" |
| `* pages you:*` | Lines with "pages you:" anywhere |
| `You feel hungry*` | Lines starting with "You feel hungry" |

## Capture Groups

When a pattern matches, you can use captured text in commands:

| Variable | Description |
|----------|-------------|
| `$0` | The entire matched text |
| `$1` - `$9` | Captured groups from the pattern |

### Regexp Capture Groups

Use parentheses to create groups:

```
Pattern: ^(\w+) tells you: (.*)$
Input:   "Bob tells you: Hello!"
$1 = "Bob"
$2 = "Hello!"
```

### Wildcard Capture Groups

Each `*` and `?` becomes a capture group automatically:

```
Pattern: * tells you: *
Input:   "Bob tells you: Hello!"
$1 = "Bob"
$2 = "Hello!"
```

## Command Examples

### Basic Auto-Response

```
Name: tell_thanks
Pattern: * tells you: *
Command: tell $1 Thanks for the message!
```

### Multi-Command Action

Separate commands with semicolons:

```
Name: heal_self
Pattern: Your health drops to *
Command: cast heal;drink potion;say Ouch!
```

### Notification Action

```
Name: page_alert
Pattern: *pages you*
Command: /notify Page received: $0
```

## Gagging (Hiding Lines)

Use `/gag` in the command to hide matched lines:

```
Name: hide_spam
Pattern: You hear a loud noise*
Command: /gag
```

**Gagging behavior:**
- Gagged lines are hidden from normal display
- Gagged lines are still stored (visible with F2/Show tags)
- What you type is kept too, for `/recall -i` and the log, but F2 never shows it
- Useful for filtering spam while preserving history

### Combined Gag and Command

```
Name: quiet_combat
Pattern: *misses you*
Command: /gag;#set misses %{misses}+1
```

## Rewriting Lines (/substitute)

Use `/substitute <text>` in the command to show the matched line as `<text>` instead.
`$0`-`$9` stand for the captures, as in any command, and each keeps the colours it had
in the MUD's line:

```
Name: no_titles
Match Type: Regexp
Pattern: ^(?:AlwaysGreat|The) (\S+ says '.*)$
Command: /substitute $1
```

`AlwaysGreat Ztheman says 'hi'` is then shown as `Ztheman says 'hi'`.

**Rewriting behavior:**
- The rewritten line takes the line's place; it is one line, not a copy
- What the MUD sent is kept with it: **F2** shows it in the rewrite's place
- `/recall` searches and shows the rewrite; `/recall -ao` searches and shows what the
  MUD sent (`-ago` adds gagged lines too). `clay --grep ... -o` does the same from the
  command line
- The archive and the per-world log keep both: the log writes the rewrite, then the
  MUD's text on the next line after `(original) `
- Keep the pattern narrow: a pattern that drops a leading `The ` alone would turn
  "The door opens." into "door opens." too
- The text can't hold a `;`, which separates commands
- An action run by name (`/no_titles`) has no line to rewrite, so its `/substitute` is
  skipped

TinyFugue's `/substitute` in a `/def` trigger works the same way (see TinyFugue Commands).

## Manual Invocation

Actions can be invoked manually by typing `/actionname`:

```
/heal_self              # Run the heal_self action
/greet Bob              # Run greet with $1="Bob"
```

**For manual actions:**
- `$1-$9` are space-separated arguments
- `$*` is all arguments combined
- Actions with empty patterns are manual-only

### Example Manual Action

```
Name: greet
Pattern: (empty)
Command: bow $1;say Hello, $1!
```

Usage: `/greet Alice` sends "bow Alice" and "say Hello, Alice!"

## F8 Pattern Highlighting

Press `F8` to highlight lines matching any action pattern:

- Matched lines get a dark background color
- Useful for debugging action patterns
- Commands are NOT executed in highlight mode

## World-Specific Actions

Set the World field to restrict an action to one world:

```
Name: mymud_heal
World: MyMUD
Pattern: You are bleeding
Command: bandage
```

This action only triggers for output from the "MyMUD" world.

## Best Practices

1. **Test patterns first**: Use F8 highlighting to verify matches
2. **Use specific patterns**: Avoid overly broad patterns like `*`
3. **Order by specificity**: More specific patterns should have higher priority
4. **Gag carefully**: You can always see gagged lines with F2
5. **Name descriptively**: Action names become manual commands

## Common Action Recipes

### Auto-Heal When Low

```
Name: auto_heal
Pattern: HP: (\d+)/
Match: Regexp
Command: #if ($1 < 50) cast heal
```

### Reply to Tells

```
Name: afk_reply
Pattern: * tells you: *
Command: tell $1 I'm AFK, back soon!
```

### Channel Logger

```
Name: log_ooc
Pattern: \[OOC\] *
Match: Regexp
Command: #echo [Logged] $0
```

### Combat Spam Filter

```
Name: filter_misses
Pattern: * misses *
Command: /gag
```

\newpage

