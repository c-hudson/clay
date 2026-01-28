# VHS Tape: TinyFugue Commands
# Demonstrates TF compatibility commands

Output docs/images/tui/tf-commands.gif

Set Shell "bash"
Set FontSize 14
Set Width 1200
Set Height 600
Set Theme "Dracula"

# Use test config with HHGTTG character names
Type "./target/x86_64-unknown-linux-musl/debug/clay --conf=docs/test-clay.dat"
Enter
Sleep 3s

# Demonstrate #set command
Type "#set myvar hello"
Enter
Sleep 500ms
Screenshot docs/images/tui/tf-set.png
Sleep 500ms

# Demonstrate #echo
Type "#echo Value is %{myvar}"
Enter
Sleep 500ms

# Demonstrate #def
Type "#def greet = say Hello, world!"
Enter
Sleep 500ms
Screenshot docs/images/tui/tf-def.png
Sleep 500ms

Type "/quit"
Enter
Sleep 500ms
