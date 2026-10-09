#!/bin/bash
# The native GUI's windows against a live server, offscreen: the snapshot harness drives
# them (clicks, typing, `pump` to let the server answer) and settings.dat is checked
# afterwards. Notes, Settings, a new action, a world's port, and the three editors.
#   tests/native_gui/roundtrip.sh <outdir>
# Needs target/native-gui/debug/clay built with NATIVE_GUI_FEATURES=native-gui-snapshot.
# Click positions are the windows' logical pixels at their default sizes.
set -u
OUT=${1:?usage: roundtrip.sh <outdir>}
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN=${BIN:-$ROOT/target/native-gui/debug}
MUD=19711; WEB=19712
mkdir -p "$OUT"
run() {   # run <name> <window> <steps> <pattern> [file under ~/.clay, default settings.dat]
    local WORK; WORK=$(mktemp -d)
    mkdir -p "$WORK/home/.clay"; chmod 700 "$WORK/home/.clay"
    printf '[global]\nhttp_enabled=true\nhttp_port=%s\nmore_mode=false\n\n[world:Echo]\nworld_type=mud\nhostname=127.0.0.1\nport=%s\nuse_ssl=false\n\n[action:echo-connect]\ncommand=/connect Echo\nstartup=true\n' $WEB $MUD > "$WORK/home/.clay/settings.dat"
    python3 -I "$(dirname "$0")/echo-mud.py" $MUD "$WORK/mud.log" & local mud=$!
    HOME=$WORK/home CLAY_WS_PASSWORD=bench "$BIN/clay" --local-server --port=$WEB > "$WORK/ls.log" 2>&1 & local ls=$!
    sleep 2
    HOME=$WORK/home CLAY_PASSWORD=bench CLAY_SNAPSHOT_WINDOW="$2" CLAY_SNAPSHOT_INPUT="$3" "$BIN/clay" --native-gui=127.0.0.1:$WEB --native-gui-snapshot="$OUT/$1.png" > "$WORK/snap.log" 2>&1
    sleep 0.5
    local file="$WORK/home/.clay/${5:-settings.dat}"
    if grep -qP -- "$4" "$file" 2>/dev/null; then echo "PASS $1"; else echo "FAIL $1: $(grep -E 'snapshot|panic|error' $WORK/snap.log | head -2)"; cp "$file" "$OUT/$1.saved" 2>/dev/null; fi
    kill $mud $ls 2>/dev/null; wait $mud $ls 2>/dev/null
    rm -rf "$WORK"
}
run notes notes "open:notes;pump:800;win:notes;text:remember the troll;down:578,12;up:578,12;pump:1000" "notes=remember the troll"
run settings settings "open:setup;win:settings;down:167,275;up:167,275;down:473,654;up:473,654;pump:1000" "^more_mode=true"
run action actions "open:actions;win:actions;down:590,591;up:590,591;text:greet;down:300,450;up:300,450;text:say hello;down:719,593;up:719,593;pump:1000" "^\[action:greet\]"
run world worlds "open:world-selector;win:worlds;down:420,652;up:420,652;down:400,174;up:400,174;key:end;key:backspace;key:backspace;key:backspace;key:backspace;key:backspace;text:4321;down:494,652;up:494,652;pump:1000" "^port=4321"
# The editors: a key captured for Cursor Left, then Save; a colour picked for bg, then Save;
# a new action named over its selected placeholder name, then Save.
run keybind keybind "open:keybind-editor;pump:800;win:keybind;down:100,90;up:100,90;down:600,262;up:600,262;key:f7;pump:500;down:771,22;up:771,22;pump:1000" "F7" keybindings.dat
run theme theme "open:theme-editor;pump:800;win:theme;down:100,95;up:100,95;down:440,100;up:440,100;wait:100;pump:600;down:785,24;up:785,24;pump:1000" "^bg = #(?!131926)" theme.dat
run actioneditor action "open:action-editor;pump:800;win:action;down:140,140;up:140,140;text:heal;down:910,603;up:910,603;pump:1000" "^\[action:heal\]"
# Closing the Action editor with an unsaved new action asks first; its Save saves, and the
# window closes (so the main window is the one photographed).
run actionclose "" "open:action-editor;pump:800;win:action;down:140,140;up:140,140;text:heal;close:;down:400,363;up:400,363;pump:1000" "^\[action:heal\]"
