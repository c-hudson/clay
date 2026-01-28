# VHS Tape: Spell Checking
# Demonstrates spell checking feature

Output docs/images/tui/spell-check.gif

Set Shell "bash"
Set FontSize 14
Set Width 1200
Set Height 600
Set Theme "Dracula"

# Use test config with HHGTTG character names
Type "./target/x86_64-unknown-linux-musl/debug/clay --conf=docs/test-clay.dat"
Enter
Sleep 3s

# Type a misspelled word
Type "helo wrold "
Sleep 1s
Screenshot docs/images/tui/spell-check.png
Sleep 500ms

# Clear and quit
Ctrl+U
Type "/quit"
Enter
Sleep 500ms
