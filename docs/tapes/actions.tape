# VHS Tape: Clay Actions Editor
# Captures actions list and editor

Output docs/images/tui/actions.gif

Set Shell "bash"
Set FontSize 14
Set Width 1200
Set Height 600
Set Theme "Dracula"

# Use test config with HHGTTG character names
Type "./target/x86_64-unknown-linux-musl/debug/clay --conf=docs/test-clay.dat"
Enter
Sleep 3s

# Actions list popup
Type "/actions"
Enter
Sleep 1s
Screenshot docs/images/tui/actions-list.png
Sleep 500ms

# Open action editor (Add)
Type "A"
Sleep 1s
Screenshot docs/images/tui/actions-editor.png
Sleep 500ms

Escape
Sleep 500ms
Escape
Sleep 500ms
Type "/quit"
Enter
Sleep 500ms
