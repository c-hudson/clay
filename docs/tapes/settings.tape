# VHS Tape: Clay Settings Popups
# Captures setup and web settings

Output docs/images/tui/settings.gif

Set Shell "bash"
Set FontSize 14
Set Width 1200
Set Height 600
Set Theme "Dracula"

# Use test config with HHGTTG character names
Type "./target/x86_64-unknown-linux-musl/debug/clay --conf=docs/test-clay.dat"
Enter
Sleep 3s

# Global Settings popup
Type "/setup"
Enter
Sleep 1s
Screenshot docs/images/tui/setup.png
Sleep 500ms

# Close and open web settings
Escape
Sleep 500ms
Type "/web"
Enter
Sleep 1s
Screenshot docs/images/tui/web.png
Sleep 500ms

Escape
Sleep 500ms
Type "/quit"
Enter
Sleep 500ms
