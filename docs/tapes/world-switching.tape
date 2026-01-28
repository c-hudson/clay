# VHS Tape: World Switching
# Demonstrates multi-world navigation

Output docs/images/tui/world-switching.gif

Set Shell "bash"
Set FontSize 14
Set Width 1200
Set Height 600
Set Theme "Dracula"

# Use test config with HHGTTG character names
Type "./target/x86_64-unknown-linux-musl/debug/clay --conf=docs/test-clay.dat"
Enter
Sleep 3s

# Create a new world via /worlds
Type "/worlds Vogosphere"
Enter
Sleep 1s

# Fill in some details in the editor
Type "vogosphere.example.com"
Down
Type "4201"
Sleep 500ms
Screenshot docs/images/tui/world-switching.png
Sleep 500ms

Escape
Sleep 500ms
Type "/quit"
Enter
Sleep 500ms
