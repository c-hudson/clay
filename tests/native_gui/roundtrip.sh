#!/bin/bash
# The native GUI's windows against a live server, offscreen: the snapshot harness drives
# them (clicks, typing, `pump` to let the server answer) and settings.dat is checked
# afterwards. Notes, Settings, a new action, a world's port.
#   tests/native_gui/roundtrip.sh <outdir>
# Needs target/native-gui/debug/clay built with NATIVE_GUI_FEATURES=native-gui-snapshot.
# Click positions are the windows' logical pixels at their default sizes.
set -u
OUT=${1:?usage: roundtrip.sh <outdir>}
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN=${BIN:-$ROOT/target/native-gui/debug}
MUD=19711; WEB=19712
mkdir -p "$OUT"
run() {   # run <name> <window> <steps> <pattern>
    local WORK; WORK=$(mktemp -d)
    mkdir -p "$WORK/home/.clay"; chmod 700 "$WORK/home/.clay"
    printf '[global]\nhttp_enabled=true\nhttp_port=%s\nmore_mode=false\n\n[world:Echo]\nworld_type=mud\nhostname=127.0.0.1\nport=%s\nuse_ssl=false\n\n[action:echo-connect]\ncommand=/connect Echo\nstartup=true\n' $WEB $MUD > "$WORK/home/.clay/settings.dat"
    python3 -I "$(dirname "$0")/echo-mud.py" $MUD "$WORK/mud.log" & local mud=$!
    HOME=$WORK/home CLAY_WS_PASSWORD=bench "$BIN/clay" --local-server --port=$WEB > "$WORK/ls.log" 2>&1 & local ls=$!
    sleep 2
    HOME=$WORK/home CLAY_PASSWORD=bench CLAY_SNAPSHOT_WINDOW="$2" CLAY_SNAPSHOT_INPUT="$3" "$BIN/clay" --native-gui=127.0.0.1:$WEB --native-gui-snapshot="$OUT/$1.png" > "$WORK/snap.log" 2>&1
    sleep 0.5
    if grep -q -- "$4" "$WORK/home/.clay/settings.dat"; then echo "PASS $1"; else echo "FAIL $1: $(grep -E 'snapshot|panic|error' $WORK/snap.log | head -2)"; cp "$WORK/home/.clay/settings.dat" "$OUT/$1.settings"; fi
    kill $mud $ls 2>/dev/null; wait $mud $ls 2>/dev/null
    rm -rf "$WORK"
}
run notes notes "open:notes;pump:800;win:notes;text:remember the troll;down:578,12;up:578,12;pump:1000" "notes=remember the troll"
run settings settings "open:setup;win:settings;down:167,275;up:167,275;down:473,654;up:473,654;pump:1000" "^more_mode=true"
run action actions "open:actions;win:actions;down:590,591;up:590,591;text:greet;down:300,450;up:300,450;text:say hello;down:719,593;up:719,593;pump:1000" "^\[action:greet\]"
run world worlds "open:world-selector;win:worlds;down:420,652;up:420,652;down:400,174;up:400,174;key:end;key:backspace;key:backspace;key:backspace;key:backspace;key:backspace;text:4321;down:494,652;up:494,652;pump:1000" "^port=4321"
