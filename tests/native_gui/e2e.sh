#!/bin/bash
# End-to-end output-sync checks for the native GUI's client: a real Clay server
# (`clay --local-server`) connected to reference/fake-mud-server.py (bursts of lines,
# escape-only lines, blanks and partial prompts every 150 ms), and the client running
# headless (`--gui-verify`), which checks its buffer against the server's.
#   tests/native_gui/e2e.sh [scenario...]     (default: all)
# Scenarios: baseline, drops (the client's connection cut three times mid-burst),
# reload (the server hot-reloads mid-burst), more (more-mode holding output back).
# Needs target/native-gui/debug/clay built with --features native-gui.
set -u
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
CLAY=${CLAY_NATIVE:-$ROOT/target/native-gui/debug/clay}
SECS=${E2E_SECS:-30}
MUD=19361; WEB=19362
PASS=0; FAIL=0

run() {   # run <name> <more_mode> <actions>
    local name=$1 more=$2 actions=$3
    local work; work=$(mktemp -d)
    mkdir -p "$work/home/.clay"; chmod 700 "$work/home/.clay"
    cat > "$work/home/.clay/settings.dat" <<EOF
[global]
http_enabled=true
http_port=$WEB
more_mode=$more

[world:Fake]
world_type=mud
hostname=127.0.0.1
port=$MUD
use_ssl=false

[action:e2e-connect]
command=/connect Fake
startup=true
EOF
    python3 -I "$ROOT/reference/fake-mud-server.py" $MUD ${E2E_MUD_FLAGS:-} > "$work/mud.log" 2>&1 &
    local mud=$!
    sleep 0.5
    HOME=$work/home CLAY_WS_PASSWORD=e2e "$CLAY" --local-server --port=$WEB > "$work/server.log" 2>&1 &
    local server=$!
    sleep 3
    echo "=== $name ==="
    if HOME=$work/client CLAY_PASSWORD=e2e CLAY_VERIFY_ACTIONS="$actions" \
        "$CLAY" --gui=127.0.0.1:$WEB --gui-verify="$SECS" > "$work/verify.log" 2>&1; then
        PASS=$((PASS + 1))
    else
        FAIL=$((FAIL + 1))
    fi
    cat "$work/verify.log"
    # Only our own processes: the server may have re-exec'd itself (same pid) on reload.
    kill $server $mud 2>/dev/null
    wait $server $mud 2>/dev/null
    rm -rf "$work"
}

scenarios=${*:-baseline drops reload more}
for s in $scenarios; do
    case $s in
        baseline) run baseline false "" ;;
        drops) run drops false "6:drop;13:drop;20:drop" ;;
        reload) run reload false "10:/reload" ;;
        more) run more true "" ;;
        *) echo "unknown scenario $s"; FAIL=$((FAIL + 1)) ;;
    esac
done
echo "e2e: $PASS passed, $FAIL failed"
[ $FAIL -eq 0 ]
