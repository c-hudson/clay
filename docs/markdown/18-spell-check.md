# Spell Checking

Clay includes real-time spell checking with suggestions, using the system dictionary.

## Dictionary

### Location

Clay looks for the system dictionary at:
1. `/usr/share/dict/words`
2. `/usr/share/dict/american-english` (fallback)
3. `/usr/share/dict/british-english` (fallback)

### Dictionary Format

- Plain text file, one word per line
- Case-insensitive matching
- Typically 100,000+ words

### Installing Dictionary

If dictionary is not installed:

```bash
# Debian/Ubuntu
sudo apt install wamerican

# Or British English
sudo apt install wbritish

# Fedora/RHEL
sudo dnf install words

# macOS
# Dictionary exists at /usr/share/dict/words by default
```

## Configuration

### Enable/Disable

1. Open `/setup`
2. Toggle "Spell check"

Or with TinyFugue commands:
```
#set spell_check on
#set spell_check off
```

## How It Works

### Word Checking

- Words are only checked when "complete" (followed by space/punctuation)
- Words at end of input are NOT checked while typing
- Prevents premature flagging of partial words

### Visual Feedback

- Misspelled words highlighted in red
- Highlighting persists until word is re-checked

### Caching

Misspelling state is cached between keystrokes:
- Prevents flickering when editing
- Example: "thiss " → flagged → backspace to "thiss" → stays flagged → backspace to "this" → stays flagged until space, then re-checked

## Suggestions

Press `Ctrl+Q` to get spell suggestions:

1. Place cursor on or after a misspelled word
2. Press `Ctrl+Q`
3. First suggestion replaces the word
4. Press `Ctrl+Q` again to cycle through suggestions

### Suggestion Algorithm

Uses Levenshtein distance:
- Maximum edit distance: 3
- Prefers words of similar length
- Sorted by edit distance (closest first)

## Contraction Support

Contractions are recognized as valid words:
- "didn't", "won't", "I'm", "you're"
- Apostrophes between alphabetic characters are part of the word
- Special handling for irregular contractions (e.g., "won't" → "will")

## Examples

### Typing Flow

```
Type: "helo "
      ^^^^
      Red (misspelled)

Type: "hello "
      ^^^^^
      Normal (correct)
```

### Using Suggestions

```
Type: "recieve "
      ^^^^^^^
      Red (misspelled)

Press Ctrl+Q:
      "receive "
      ^^^^^^^
      Normal (replaced with suggestion)
```

### Cycling Suggestions

```
Word: "teh"
Ctrl+Q → "the"
Ctrl+Q → "tea"
Ctrl+Q → "ten"
...
```

## Limitations

### Not Checked

- Command prefixes (`/`, `#`)
- URLs and paths
- Words with numbers
- Very short words (1-2 characters)

### Dictionary Limitations

- Technical terms may be flagged
- Proper nouns may be flagged
- MUD-specific vocabulary not included

### Performance

- Large dictionaries load quickly
- Suggestions computed on-demand
- Caching minimizes re-checking

## Integration

### Works With

- Console input
- Web interface input (if enabled)
- GUI input (if enabled)

### Doesn't Work With

- Output text (not checked)
- Popup text fields (settings, etc.)

## Technical Details

### SpellChecker Struct

```rust
struct SpellChecker {
    words: HashSet<String>,
    // Dictionary loaded at startup
}
```

### Levenshtein Distance

Edit distance between two strings:
- Insertion: +1
- Deletion: +1
- Substitution: +1

Maximum distance of 3 balances:
- Finding reasonable suggestions
- Performance (limiting search space)

### Word Boundaries

A word is considered complete when followed by:
- Space
- Tab
- Punctuation (. , ! ? ; : etc.)
- End of line (when checking mid-input)

## Troubleshooting

### No Spell Checking

1. Verify enabled in `/setup`
2. Check dictionary file exists
3. Check dictionary file is readable

### Too Many False Positives

1. Consider adding custom dictionary
2. Use `/setup` to disable if too distracting
3. Technical content may not match dictionary

### Suggestions Not Working

1. Verify cursor is on/near misspelled word
2. Check word is actually flagged (red)
3. Try pressing `Ctrl+Q` multiple times

### Missing Dictionary

```bash
# Check if dictionary exists
ls -la /usr/share/dict/words

# Install if missing
sudo apt install wamerican
```

\newpage

