# The Map

Clay draws a map of each world as you move through it, from the rooms the game reports
over GMCP. Pick a room on the map and Clay shows the way there; ask it to walk and it
sends the steps for you, one room at a time.

The map is built on the machine running Clay, so every interface shows the same map: the
console, the SSH remote console, the GUI, the web page and the Android app.

## What the game has to send

The map needs a game that reports the room you are in over GMCP (the `Room.Info`
message, IRE's and most Diku games' shape; Aardwolf's `room.info`; or one with `x`, `y`
and `z` on the room). A game that sends no room data gets no map: Clay does not guess
rooms from the text on the screen.

The world's GMCP field (World Editor) asks for room reports with `Room 1`. A new world
asks by default (`Client.Media 1, Char 1, Room 1`), and so does an existing world that
was on the default. A world whose GMCP field you changed is left as you set it: add
`Room 1` to it yourself. A connection made before the map existed is asked for room
reports the first time you open its map.

## Opening the map

Type `/map`, or choose **Map** in the menu (the ☰ button in the GUI, the web page and the
app). `/map` again closes it.

- **The console** draws the map above the output, two fifths of the output area at most,
  with a bar under it naming the area, the level and how many rooms the map has.
- **The GUI and the web page** show it in a pane on the right of the output.
- **A phone** shows it as a sheet over the lower part of the output, so the output never
  changes size or rewraps.

Each interface keeps its own pane, open or closed, as with `/split`. A `/window` has no
map pane: the map is in the main window.

## Reading the map

| Shown as | Meaning |
|----------|---------|
| A square (`[ ]` in the console) | A room you have been to |
| The accent colour (`[@]`) | The room you are in |
| Yellow (`[X]`) | The room picked, or the one a walk is going to |
| A yellow line | The route there |
| A dashed line (`·`) | An exit nobody has taken yet |
| A short coloured line (`>`) | An exit to another area |
| ▲ ▼ (`^` `v`) | A way up or down |

Each area is its own map, and each level (up and down) its own layer. The map shows the
area and level you are in; the level buttons (▴ ▾) or `/map level up` and
`/map level down` show the others, and the ◎ button or `/map center` comes back to you.
In the GUI and the web page, drag the map to move it and use the mouse wheel or the
+ and − buttons to zoom.

Rooms are placed by the game's own coordinates when it sends them. Otherwise a room is
placed beside the room you came from, in the direction you went. Once placed, a room
never moves by itself, so the map doesn't shift under you as it grows.

## Going somewhere

Click a room, or find one by name in the search box, and its card shows the room's
exits and the route from where you are. **Walk here** walks it. From any interface, the
commands do the same:

```
/map path Temple       # the route, without walking it
/map go Temple         # walk there
/map go #1234          # a room by the game's own number
```

A name is matched whole first, then by its start, then anywhere in the name; among rooms
with the same name, the nearest is taken.

A walk sends one step, waits for the game to report the room it leads to, then sends the
next. It stops by itself, saying why, when:

- the game reports a room the route didn't expect (a door, a teleport, a monster that
  moved you): `Walk stopped: expected Dusty Road (#1235) after 'e'; you are in Muddy
  Track (#2210).`
- the game says it can't go that way (GMCP `Room.WrongDir`);
- no room report comes within 10 seconds of a step;
- the world disconnects.

**Esc**, the card's **Stop** button or `/map stop` stops a walk at once. Esc still does
everything else it does, so Esc-w still switches worlds. Typing your own commands does
not stop a walk; a move of your own stops it by taking you somewhere it didn't expect.
A walk carries on through a `/reload`.

## Commands

| Command | What it does |
|---------|--------------|
| `/map` | Show or hide the map |
| `/map go <room or #id>` | Walk there, one step per room the game reports |
| `/map path <room or #id>` | Show the route, without walking it |
| `/map stop` | Stop walking (Esc does too) |
| `/map find <text>` | List the rooms whose name has it |
| `/map here` | Say the room you are in |
| `/map area [name]` | Show another area's map (no name: yours) |
| `/map level <n\|up\|down>` | Show another level |
| `/map center` | Back to the room you are in |
| `/map forget <world>` | Delete that world's whole map |

TinyFugue's `map.tf` library defines a `/map` macro of its own. While it is loaded, its
macro wins, and `/@map` reaches Clay's.

## Where maps are kept

Maps are saved in `~/.clay/maps.db` (`~/clay/maps.db` on Windows), one per world. A map
belongs to the world, not to its address: renaming a world keeps its map, and two worlds
on the same game have a map each. A deleted world's map is kept for 30 days after it last
changed, in case the world comes back, then deleted. `/map forget <world>` deletes one
straight away.

On `clay_multi` each user has a map of their own, kept in their own directory on the
server; `clay_web`'s visitors have none.

## With a screen reader

The search box, the card and its buttons are labelled controls. `/map path` and `/map go`
are the keyboard way to everything the map does: the route is read out as a line, and so
is each stop and arrival.
