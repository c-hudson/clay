#!/bin/bash
# macOS WebView side of the native GUI's Phase-0 comparison: shows each corpus page in
# the WebView GUI for a few seconds and screenshots the screen. Run it from Terminal in
# the logged-in desktop session (a GUI started over SSH gets no window on macOS). The
# native side is rendered offscreen by snapshot.sh.
#   tests/native_gui/bench-mac.sh [outdir]      (default ~/nbench/mac-web)
# If the screenshots show only the desktop, allow Terminal under System Preferences >
# Security & Privacy > Privacy > Screen Recording, then run it again.
set -u
OUT=${1:-$HOME/nbench/mac-web}
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN=${CLAY_BIN_DIR:-$ROOT/target/debug}
WORK=$(mktemp -d)
MUD=19311; WEB=19312
mkdir -p "$OUT"
i=0
while IFS= read -r page; do
    i=$((i + 1))
    rm -rf "$WORK/home"; mkdir -p "$WORK/home/.clay"; chmod 700 "$WORK/home/.clay"
    cat > "$WORK/home/.clay/settings.dat" <<EOF
[global]
http_enabled=true
http_port=$WEB
more_mode=false

[world:Corpus]
world_type=mud
hostname=127.0.0.1
port=$MUD
use_ssl=false

[action:bench-connect]
command=/connect Corpus
startup=true
EOF
    CLAY_CORPUS_SECTIONS="$page" "$BIN/clay-test-server" --ports=$MUD-$MUD --scenarios=corpus > "$WORK/ts.log" 2>&1 &
    ts=$!
    sleep 1
    HOME=$WORK/home "$BIN/clay" --web-gui > "$WORK/gui.log" 2>&1 &
    gui=$!
    sleep 10
    screencapture -x "$OUT/page$i.png"
    echo "page $i: $OUT/page$i.png"
    kill $gui $ts 2>/dev/null; wait $gui $ts 2>/dev/null
    sleep 1
done <<'PAGES'
Prose,16 colours
256 colours,Truecolour,Attributes
Block art,Box drawing
CJK,Emoji
Long words and URLs,Combining marks and scripts,Tabs and spacing,Discord and MUD tags
PAGES
rm -rf "$WORK"
