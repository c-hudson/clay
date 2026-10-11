#!/bin/bash
# Offscreen native-GUI snapshots of the rendering corpus (Linux and macOS: GPUI's
# headless renderer), one PNG per page. Needs a build with --features native-gui-snapshot.
#   tests/native_gui/snapshot.sh <outdir>
# Opens no window: test server -> `clay --local-server` -> `clay --gui-snapshot`.
set -u
OUT=${1:?usage: snapshot.sh <outdir>}
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN=${CLAY_BIN_DIR:-$ROOT/target/debug}
WORK=$(mktemp -d)
MUD=19301; WEB=19302
mkdir -p "$OUT" "$WORK/home/.clay"; chmod 700 "$WORK/home/.clay"
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
i=0
while IFS= read -r page; do
    i=$((i + 1))
    CLAY_CORPUS_SECTIONS="$page" "$BIN/clay-test-server" --ports=$MUD-$MUD --scenarios=corpus > "$WORK/ts.log" 2>&1 &
    ts=$!
    HOME=$WORK/home CLAY_WS_PASSWORD=bench "$BIN/clay" --local-server --port=$WEB > "$WORK/ls.log" 2>&1 &
    ls_pid=$!
    sleep 3
    HOME=$WORK/home CLAY_PASSWORD=bench "$BIN/clay" --gui=127.0.0.1:$WEB --gui-snapshot="$OUT/page$i.png" 2>&1 | grep snapshot
    kill $ts $ls_pid 2>/dev/null; wait $ts $ls_pid 2>/dev/null
    rm -rf "$WORK/home/.clay/logs" "$WORK/home/.clay/scrollback.db"
done <<'PAGES'
Prose,16 colours
256 colours,Truecolour,Attributes
Block art,Box drawing
CJK,Emoji
Long words and URLs,Combining marks and scripts,Tabs and spacing,Discord and MUD tags
PAGES
rm -rf "$WORK"
