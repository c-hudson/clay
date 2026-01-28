# VHS Tape: Connect to World
# Shows the connection process

Output docs/images/tui/connect-world.gif

Set Shell "bash"
Set FontSize 14
Set Width 1200
Set Height 600
Set Theme "Dracula"

# Use test config with HHGTTG character names
Type "./target/x86_64-unknown-linux-musl/debug/clay --conf=docs/test-clay.dat"
Enter
Sleep 3s

# Open world selector
Type "/worlds"
Enter
Sleep 1s

# Navigate and show interface
Down
Sleep 500ms
Screenshot docs/images/tui/connect-world.png
Sleep 500ms

Escape
Sleep 500ms
Type "/quit"
Enter
Sleep 500ms
