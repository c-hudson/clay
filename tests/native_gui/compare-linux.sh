#!/bin/bash
# Linux Phase-0 comparison, entirely offscreen: for each corpus page, one `clay
# --local-server` that both the WebView (WebKitGTK under a headless broadway display,
# webview-snapshot-gtk.py) and the native GUI (--gui-snapshot) attach to, each
# rendered at 2x. Then compare.py on every pair.
#   tests/native_gui/compare-linux.sh <outdir>
# Caveat: WebKitGTK's zoom rounds glyph advances, so at 2x its text drifts along a long
# line and the pixel comparison is skewed; for judging glyphs, bench.sh's 1x captures of
# real windows are the fair comparison. This is still useful for wraps and layout.
# Needs target/native-gui/debug/clay built with --features native-gui-snapshot.
set -u
OUT=${1:?usage: compare-linux.sh <outdir>}
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
NATIVE=${CLAY_NATIVE:-$ROOT/target/native-gui/debug/clay}
TESTSRV=${CLAY_TEST_SERVER:-$ROOT/target/debug/clay-test-server}
WORK=$(mktemp -d)
MUD=19331; WEB=19332; DISPLAY_NO=${BROADWAY_NO:-23}
HASH=$(printf bench | sha256sum | cut -d' ' -f1)
mkdir -p "$OUT/web" "$OUT/native" "$WORK/home/.clay"; chmod 700 "$WORK/home/.clay"
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
    broadwayd ":$((DISPLAY_NO + i))" > "$WORK/broadway.log" 2>&1 &
    bw=$!
    CLAY_CORPUS_SECTIONS="$page" "$TESTSRV" --ports=$MUD-$MUD --scenarios=corpus > "$WORK/ts.log" 2>&1 &
    ts=$!
    HOME=$WORK/home CLAY_WS_PASSWORD=bench "$NATIVE" --local-server --port=$WEB > "$WORK/ls.log" 2>&1 &
    ls_pid=$!
    sleep 3
    GDK_BACKEND=broadway BROADWAY_DISPLAY=":$((DISPLAY_NO + i))" python3 "$ROOT/tests/native_gui/webview-snapshot-gtk.py" \
        "http://127.0.0.1:$WEB/" "$OUT/web/page$i.png" "$HASH" 800 600 8 2
    HOME=$WORK/home CLAY_PASSWORD=bench "$NATIVE" --gui=127.0.0.1:$WEB --gui-snapshot="$OUT/native/page$i.png" 2>&1 | grep snapshot
    kill $ts $ls_pid $bw 2>/dev/null; wait $ts $ls_pid $bw 2>/dev/null
    rm -rf "$WORK/home/.clay/logs" "$WORK/home/.clay/scrollback.db"
done <<'PAGES'
Prose,16 colours
256 colours,Truecolour,Attributes
Block art,Box drawing,FANSI symbols
CJK,Emoji
Long words and URLs,Combining marks and scripts,Tabs and spacing,Discord and MUD tags
PAGES
rm -rf "$WORK"
