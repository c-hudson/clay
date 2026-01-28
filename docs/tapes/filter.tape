# VHS Tape: Filter Popup
# Captures the F4 filter popup

Output docs/images/tui/filter.gif

Set Shell "bash"
Set FontSize 14
Set Width 1200
Set Height 600
Set Theme "Dracula"

# Use test config with HHGTTG character names
Type "./target/x86_64-unknown-linux-musl/debug/clay --conf=docs/test-clay.dat"
Enter
Sleep 3s

# Redraw screen with Ctrl+L
Ctrl+L
Sleep 500ms
Screenshot docs/images/tui/filter.png
Sleep 500ms

Type "/quit"
Enter
Sleep 500ms
