# VHS Tape: Commands Demonstration
# Shows command execution and connections list

Output docs/images/tui/commands.gif

Set Shell "bash"
Set FontSize 14
Set Width 1200
Set Height 600
Set Theme "Dracula"

# Use test config with HHGTTG character names
Type "./target/x86_64-unknown-linux-musl/debug/clay --conf=docs/test-clay.dat"
Enter
Sleep 3s

# Show connections list (empty when not connected)
Type "/connections"
Enter
Sleep 1s
Screenshot docs/images/tui/connections.png
Sleep 500ms

Type "/quit"
Enter
Sleep 500ms
