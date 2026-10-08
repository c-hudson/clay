#!/bin/bash
# Side-by-side bench for the native GUI's Phase-0 rendering comparison (Linux/X11).
#
# Starts clay-test-server serving the corpus twice, then:
#   - the WebView GUI in master mode, with its own HOME (it logs itself in), and
#   - a headless `clay --local-server` with another HOME, which the native GUI attaches to.
# Both worlds connect to the corpus through a startup action, so both GUIs show the
# same output. With --capture, each window is photographed with xwd and saved as PNG.
#
# Usage: tests/native_gui/bench.sh <workdir> [--capture] [--keep]
#   CLAY_WEBVIEW  clay binary built with --features webview-gui
#   CLAY_NATIVE   clay binary built with --features native-gui
#   CLAY_TEST_SERVER  clay-test-server binary
set -u
WORK=${1:?usage: bench.sh <workdir> [--capture] [--keep]}
shift
CAPTURE=0; KEEP=0
for a in "$@"; do
    case $a in --capture) CAPTURE=1 ;; --keep) KEEP=1 ;; esac
done
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
WEBVIEW=${CLAY_WEBVIEW:-$ROOT/target/webview/debug/clay}
NATIVE=${CLAY_NATIVE:-$ROOT/target/native-gui/debug/clay}
TESTSRV=${CLAY_TEST_SERVER:-$ROOT/target/debug/clay-test-server}
MUD_A=19101; MUD_B=19102; WEB_A=19201; WEB_B=19202; PW=bench

mkdir -p "$WORK"
PIDS=()
cleanup() {
    for p in "${PIDS[@]}"; do kill "$p" 2>/dev/null; done
    # Wait for them to go: the next run binds the same ports.
    for p in "${PIDS[@]}"; do
        for _ in $(seq 1 50); do kill -0 "$p" 2>/dev/null || break; sleep 0.1; done
        kill -9 "$p" 2>/dev/null
    done
}
trap cleanup EXIT

make_home() {   # make_home <dir> <mud-port> <http-port>
    rm -rf "$1"; mkdir -p "$1/.clay"; chmod 700 "$1/.clay"
    cat > "$1/.clay/settings.dat" <<EOF
[global]
http_enabled=true
http_port=$3
more_mode=false

[world:Corpus]
world_type=mud
hostname=127.0.0.1
port=$2
use_ssl=false

[action:bench-connect]
command=/connect Corpus
startup=true
EOF
}

make_home "$WORK/home-webview" $MUD_A $WEB_A
make_home "$WORK/home-native" $MUD_B $WEB_B

"$TESTSRV" --ports=$MUD_A-$MUD_B --scenarios=corpus,corpus > "$WORK/testserver.log" 2>&1 &
PIDS+=($!)
sleep 0.5

HOME=$WORK/home-native CLAY_WS_PASSWORD=$PW "$NATIVE" --local-server --port=$WEB_B > "$WORK/localserver.log" 2>&1 &
PIDS+=($!)
HOME=$WORK/home-webview "$WEBVIEW" --gui > "$WORK/webview.log" 2>&1 &
PIDS+=($!); WEBVIEW_PID=$!
sleep 3
HOME=$WORK/home-native CLAY_PASSWORD=$PW "$NATIVE" --native-gui=127.0.0.1:$WEB_B > "$WORK/native.log" 2>&1 &
PIDS+=($!); NATIVE_PID=$!
sleep 4

window_of() {   # the top-level X window of a pid
    for id in $(xprop -root _NET_CLIENT_LIST | sed 's/.*# //; s/,//g'); do
        if [ "$(xprop -id "$id" _NET_WM_PID 2>/dev/null | awk '{print $3}')" = "$1" ]; then echo "$id"; return; fi
    done
}

if [ $CAPTURE = 1 ]; then
    for side in webview:$WEBVIEW_PID native:$NATIVE_PID; do
        name=${side%%:*}; pid=${side##*:}
        win=$(window_of "$pid")
        if [ -z "$win" ]; then echo "$name: no window found"; continue; fi
        xwd -silent -id "$win" -out "$WORK/$name.xwd" && python3 -I "$HERE/xwd2png.py" "$WORK/$name.xwd" "$WORK/$name.png"
    done
fi

if [ $KEEP = 1 ]; then
    echo "Running. Press Enter to stop."; read -r _
fi
