#!/bin/bash
# The native GUI's client against a server on another address - this machine's LAN
# address, so the server treats it as remote: TLS, the allow-list gate, CLAY-KNOCK.
#   tests/native_gui/remote.sh [lan-ip]
# 1. no auth key: the gate drops the connection (not on the allow list) -> refused
# 2. with the auth key: the client knocks, TLS, logs in with the key -> VERIFY OK
# 3. the server comes back with a new certificate: the pinned one no longer matches ->
#    refused before any login is sent
set -u
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
CLAY=${CLAY_NATIVE:-$ROOT/target/native-gui/debug/clay}
IP=${1:-$(ip -4 route get 1.1.1.1 2>/dev/null | grep -oP 'src \K[0-9.]+')}
MUD=19381; WEB=19382; KEY=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
WORK=$(mktemp -d); FAIL=0
server_home() {   # server_home <dir>
    mkdir -p "$1/.clay"; chmod 700 "$1/.clay"
    cat > "$1/.clay/settings.dat" <<INI
[global]
http_enabled=true
http_port=$WEB
more_mode=false
websocket_password=remote-pw
websocket_allow_list=10.250.250.250
websocket_auth_key=$KEY|1700000000

[world:Fake]
world_type=mud
hostname=127.0.0.1
port=$MUD
use_ssl=false

[action:e2e-connect]
command=/connect Fake
startup=true
INI
}
start_server() {  # start_server <home>
    HOME=$1 "$CLAY" -D > "$1/server.log" 2>&1 &
    SERVER=$!
    sleep 4
}
stop_server() { kill $SERVER 2>/dev/null; wait $SERVER 2>/dev/null; }
verify() {  # verify <label> <expect: ok|fail> [env...]
    local label=$1 expect=$2; shift 2
    echo "=== $label ==="
    env HOME=$WORK/client "$@" timeout 90 "$CLAY" --native-gui=$IP:$WEB --native-gui-verify=6 > "$WORK/v.log" 2>&1
    local rc=$?
    grep -E "VERIFY|connection closed|lines," "$WORK/v.log" | head -6
    if { [ $expect = ok ] && [ $rc -eq 0 ]; } || { [ $expect = fail ] && [ $rc -ne 0 ]; }; then
        echo "as expected"
    else
        echo "UNEXPECTED (exit $rc)"; FAIL=$((FAIL + 1))
    fi
}
echo "server at $IP:$WEB"
python3 -I "$ROOT/reference/fake-mud-server.py" $MUD > "$WORK/mud.log" 2>&1 &
MUDP=$!
server_home "$WORK/s1"; start_server "$WORK/s1"
verify "password only, not on the allow list" fail CLAY_PASSWORD=remote-pw E2E_DUMMY=1
grep -cE "GATE-DROP" "$WORK/s1/.clay/remote.log" | sed 's/^/  server GATE-DROP lines: /'
verify "auth key: knock, TLS, key login" ok CLAY_AUTH_KEY=$KEY
grep -E "KNOCK-OK|AUTH-SUCCESS" "$WORK/s1/.clay/remote.log" | tail -2 | sed 's/^/  server: /'
stop_server
server_home "$WORK/s2"; start_server "$WORK/s2"
verify "the server's certificate changed" fail CLAY_AUTH_KEY=$KEY
echo "  server log after the mismatch (expect no WS-AUTH):"
grep -E "WS-AUTH|AUTH-SUCCESS" "$WORK/s2/.clay/remote.log" | sed 's/^/    /' || echo "    (none)"
stop_server
kill $MUDP 2>/dev/null; wait $MUDP 2>/dev/null
rm -rf "$WORK"
echo "remote: $FAIL unexpected"
[ $FAIL -eq 0 ]
