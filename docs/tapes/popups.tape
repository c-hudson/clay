# VHS Tape: Clay Popups
# Captures various popup dialogs

Output docs/images/tui/popups.gif

Set Shell "bash"
Set FontSize 14
Set Width 1200
Set Height 600
Set Theme "Dracula"

# Use test config with HHGTTG character names
Type "./target/x86_64-unknown-linux-musl/debug/clay --conf=docs/test-clay.dat"
Enter
Sleep 3s

# World Selector popup
Type "/worlds"
Enter
Sleep 1s
Screenshot docs/images/tui/world-selector.png
Sleep 500ms

# Close and open world editor
Escape
Sleep 500ms
Type "/worlds -e"
Enter
Sleep 1s
Screenshot docs/images/tui/world-editor.png
Sleep 500ms

# Close and open help
Escape
Sleep 500ms
Type "/help"
Enter
Sleep 1s
Screenshot docs/images/tui/help.png
Sleep 500ms

Escape
Sleep 500ms
Type "/quit"
Enter
Sleep 500ms
