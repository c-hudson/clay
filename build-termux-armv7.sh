#!/usr/bin/env bash
# Cross-compile the Termux no-GUI (TUI) binary for 32-bit ARM (armv7 / armeabi-v7a)
# using the Android NDK. Unlike the aarch64 Termux binaries (built natively on-device,
# see .claude/skills/release/machines.md), there is no 32-bit device available, so this
# target is cross-compiled here on the x86_64 Linux dev box.
#
# One-time setup (see .claude/skills/release/machines.md for details):
#   - Android NDK r26d unpacked at $ANDROID_NDK_HOME (default: ~/Android/Sdk/ndk/26.3.11579264)
#   - rustup target add armv7-linux-androideabi
#   - patchelf on PATH (installed to ~/.local/bin here; no root required)
#
# Usage: ./build-termux-armv7.sh
# Output: /tmp/clay-termux-armv7-32bit-nogui

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$HOME/Android/Sdk/ndk/26.3.11579264}"
[[ -d "$ANDROID_NDK_HOME" ]] || {
    echo "error: NDK not found at $ANDROID_NDK_HOME (set ANDROID_NDK_HOME to override)" >&2
    exit 1
}

# API level for the generated clang wrapper. 24 balances reach to older 32-bit
# devices against NDK toolchain support.
API=24
TOOLCHAIN="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64"
CLANG="$TOOLCHAIN/bin/armv7a-linux-androideabi${API}-clang"

[[ -x "$CLANG" ]] || {
    echo "error: NDK clang not found at $CLANG" >&2
    exit 1
}

command -v patchelf >/dev/null || {
    echo "error: patchelf not found on PATH" >&2
    exit 1
}

# These are consumed by cargo/rustc (linker) and by cc-rs / ring's build scripts
# (rusqlite's bundled SQLite, ring's C/asm) for the armv7-linux-androideabi target.
export CARGO_TARGET_ARMV7_LINUX_ANDROIDEABI_LINKER="$CLANG"
export CC_armv7_linux_androideabi="$CLANG"
export CXX_armv7_linux_androideabi="${CLANG}++"
export AR_armv7_linux_androideabi="$TOOLCHAIN/bin/llvm-ar"

echo "Building clay for armv7-linux-androideabi (API $API)..."
cargo build --release --target armv7-linux-androideabi \
    --no-default-features --features rustls-backend

BIN="target/armv7-linux-androideabi/release/clay"
OUT="/tmp/clay-termux-armv7-32bit-nogui"

# Mirror the aarch64 Termux rpath convention (see machines.md), using the 32-bit
# system lib path instead of /system/lib64.
patchelf --set-rpath '/system/lib:/data/data/com.termux/files/usr/lib' "$BIN"

cp "$BIN" "$OUT"
echo "Done: $OUT"
file "$OUT"
