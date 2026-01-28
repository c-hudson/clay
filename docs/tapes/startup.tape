# VHS Tape: Clay Startup Screen
# Captures the initial splash screen with ASCII art logo

Output docs/images/tui/startup.gif

Set Shell "bash"
Set FontSize 14
Set Width 1200
Set Height 600
Set Theme "Dracula"

# Use test config with HHGTTG character names
Type "./target/x86_64-unknown-linux-musl/debug/clay --conf=docs/test-clay.dat"
Enter

# Wait for Clay to fully start and display splash screen
Sleep 4s
Screenshot docs/images/tui/startup.png
Sleep 500ms

Type "/quit"
Enter
Sleep 500ms
