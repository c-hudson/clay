#!/bin/bash
# Dialogs and windows side by side: each surface from the web client (WebKitGTK on a
# headless broadway display, the Linux GUI's engine, popups as the GUI's popup windows show
# them) and from the native GUI (offscreen), against one server.
#   tests/native_gui/review-dialogs.sh <outdir> [surface...]
# Writes <outdir>/web/<surface>.png and <outdir>/native/<surface>.png at matching sizes.
# Needs target/native-gui/debug/clay built with NATIVE_GUI_FEATURES=native-gui-snapshot,
# broadwayd, and PyGObject with WebKit2 4.1.
set -u
OUT=$1; shift
ONLY=" $* "
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN=$ROOT/target/native-gui/debug
HERE=$(cd "$(dirname "$0")" && pwd)
WORK=$(mktemp -d)
MUD=19761; WEB=19762; BW=:47
mkdir -p "$OUT/web" "$OUT/native" "$WORK/home/.clay"; chmod 700 "$WORK/home/.clay"
cat > "$WORK/home/.clay/settings.dat" <<EOF
[global]
http_enabled=true
http_port=$WEB
more_mode=false
${REVIEW_THEME:+gui_theme=$REVIEW_THEME}

[world:Corpus]
world_type=mud
hostname=127.0.0.1
port=$MUD
use_ssl=false
notes=Troll lair is east of the ford.\nBring fire.

[world:Second]
world_type=mud
hostname=127.0.0.1
port=$MUD
use_ssl=false

[action:connect]
command=/connect Corpus
startup=true

[action:heal]
world=Corpus
pattern.0.text=^You are bleeding
pattern.1.text=HP: (\\\\d+)/\\\\d+
command=cast heal \$1

[action:greet]
match_type=wildcard
pattern.0.text=* waves at you*
command=wave
gui_shortcut=true

[action:afk]
command=say brb; /echo afk
enabled=false
suppress_blanks=true
EOF
python3 -I "$HERE/echo-mud.py" $MUD "$WORK/mud.log" & mud=$!
HOME=$WORK/home CLAY_WS_PASSWORD=bench "$BIN/clay" --local-server --port=$WEB > "$WORK/ls.log" 2>&1 & srv=$!
broadwayd $BW > "$WORK/bw.log" 2>&1 & bw=$!
sleep 3
HASH=$(printf bench | sha256sum | cut -d' ' -f1)
want() { [ "$ONLY" = "  " ] || [[ "$ONLY" == *" $1 "* ]]; }
web() {   # web <name> <w> <h> <path> <setup js> <after js>
    GDK_BACKEND=broadway BROADWAY_DISPLAY=$BW CLAY_WEB_SETUP="$5" CLAY_WEB_AFTER="$6" \
        python3 "$ROOT/tests/native_gui/webview-snapshot-gtk.py" "http://127.0.0.1:$WEB/$4" "$OUT/web/$1.png" "$HASH" "$2" "$3" 6 2 2>&1 | grep -v '^$' | sed "s|$OUT/||" | grep -E 'snapshot|rror' | head -2
}
native() {   # native <name> <window kind or ""> <steps>
    HOME=$WORK/home CLAY_PASSWORD=bench CLAY_SNAPSHOT_WINDOW="$2" CLAY_SNAPSHOT_INPUT="$3" "$BIN/clay" --gui=127.0.0.1:$WEB --gui-snapshot="$OUT/native/$1.png" 2>&1 | sed "s|$OUT/||" | grep -E 'snapshot|panic|error' | head -2
}
type_cmd() { echo "var i=document.getElementById('input'); i.value='$1'; i.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',code:'Enter',keyCode:13,bubbles:true}));"; }
editor_login() { echo "document.getElementById('authPassword').value='bench'; doAuth(); ${1:-}"; }
popup() { echo "window.POPUP_MODE = { kind: '$1', arg: $2 };"; }

want general && { web general 520 680 "" "$(popup settings "'general'")" ""; native general settings "open:setup;wait:200"; }
want web && { web web 520 680 "" "$(popup settings "'web'")" ""; native web settings "open:web;wait:200"; }
want font && { web font 520 680 "" "$(popup settings "'font'")" ""; native font settings "open:font;wait:200"; }
want help && { web help 640 720 "" "$(popup help null)" ""; native help help "open:help;wait:200"; }
want actions && { web actions 760 620 "" "$(popup actions null)" ""; native actions actions "open:actions;wait:200"; }
want action-edit && { web action-edit 760 620 "" "$(popup actions null)" "document.getElementById('action-add-btn').click();"; native action-edit actions "open:actions;win:actions;down:590,591;up:590,591;wait:200"; }
want worlds && { web worlds 640 680 "" "$(popup worlds "{current:'Corpus'}")" ""; native worlds worlds "open:world-selector;wait:200"; }
want world-edit && { web world-edit 640 680 "" "$(popup worlds "{current:'Corpus', edit:'Corpus'}")" ""; native world-edit worlds "open:world-selector;win:worlds;down:420,652;up:420,652;wait:200"; }
want notes && { web notes 700 500 "" "window.NOTE_MODE = { world_index: 0 };" ""; native notes notes "open:notes;pump:800"; }
want menu && { web menu 800 600 "" "" "$(type_cmd /menu)"; native menu "" "open:menu;wait:200"; }
want import && { web import 800 600 "" "" "$(type_cmd /import)"; native import "" "open:import;wait:200"; }
want emoji && { web emoji 800 600 "" "" "var i=document.getElementById('input'); i.focus(); i.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',code:'Escape',keyCode:27,bubbles:true})); i.dispatchEvent(new KeyboardEvent('keydown',{key:'e',code:'KeyE',keyCode:69,bubbles:true}));"; native emoji emoji "open:emoji_picker;wait:200"; }
want theme-editor && { web theme-editor 1000 720 theme-editor "" "$(editor_login)"; native theme-editor theme "open:theme-editor;pump:800"; }
want keybind-editor && { web keybind-editor 1000 700 keybind-editor "" "$(editor_login)"; native keybind-editor keybind "open:keybind-editor;pump:800"; }
want action-editor && { web action-editor 960 700 action-editor "" "$(editor_login "setTimeout(function(){ [...document.querySelectorAll('.list-item')].find(e => e.textContent.indexOf('heal') >= 0).click(); }, 800);")"; native action-editor action "open:action-editor;pump:800;win:action;down:120,215;up:120,215;wait:200"; }

kill $mud $srv $bw 2>/dev/null; wait $mud $srv $bw 2>/dev/null
rm -rf "$WORK"
