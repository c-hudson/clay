#!/bin/bash
# cargo for the native-gui feature: the newer toolchain GPUI needs, a separate target
# directory (so these builds never thrash the release builds' target/), and on Linux a
# link directory for libxkbcommon-x11 when only its runtime library is installed (no
# -dev package).
#   tests/native_gui/cargo-native.sh build
#   tests/native_gui/cargo-native.sh test native_gui
set -e
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
TOOLCHAIN=${NATIVE_GUI_TOOLCHAIN:-1.98.1}
SUB=$1; shift
EXTRA=()
if [ "$(uname -s)" = Linux ]; then
    LINK=$ROOT/target/native-gui-link
    mkdir -p "$LINK"
    for lib in /usr/lib/x86_64-linux-gnu/libxkbcommon-x11.so.0 /usr/lib64/libxkbcommon-x11.so.0 /usr/lib/libxkbcommon-x11.so.0; do
        if [ -e "$lib" ] && ! [ -e "$(dirname "$lib")/libxkbcommon-x11.so" ]; then
            ln -sf "$lib" "$LINK/libxkbcommon-x11.so"
        fi
    done
    export RUSTFLAGS="${RUSTFLAGS:-} -L native=$LINK"
fi
exec cargo "+$TOOLCHAIN" "$SUB" --features "${NATIVE_GUI_FEATURES:-native-gui}" --target-dir "$ROOT/target/native-gui" "$@"
