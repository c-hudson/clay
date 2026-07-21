#!/usr/bin/env bash
# Cross-compile the headless Clay server binary for 64-bit ARM Android
# (arm64-v8a) using the Android NDK, for bundling inside the Android app's
# APK as android/app/src/main/jniLibs/arm64-v8a/libclay.so. The app spawns
# this as a child process (see --local-server in src/daemon.rs) so it can
# run a full Clay instance on-device with no remote server.
#
# Unlike the Termux aarch64 binaries (built natively on-device inside Termux,
# see .claude/skills/release/machines.md), this binary runs as an ordinary
# Android app subprocess — it links only against the standard system
# libraries every app process can resolve (libc.so, libm.so, etc. via
# Android's public linker namespace), so no rpath patching is needed the way
# the Termux builds require for Termux's separate userland.
#
# No-GUI (rustls-backend): the local-server mode never opens a native
# WebView (the Android app's own WebView is the client), so webview-gui
# (wry/tao/webkit2gtk/X11) is neither needed nor available for this target.
# ssh-transport IS included: the same binary also serves as the --ssh-proxy
# subprocess SshProxyManager.java launches for the Connection Settings SSH
# tunnel option (see src/ssh.rs's run_ssh_proxy_mode doc comment) - it's the
# identical "app execs its own bundled .so" mechanism --local-server already
# uses, just with different arguments, so no separate binary is needed.
#
# One-time setup (see .claude/skills/release/machines.md for details):
#   - Android NDK r26d unpacked at $ANDROID_NDK_HOME (default: ~/Android/Sdk/ndk/26.3.11579264)
#   - rustup target add aarch64-linux-android
#
# Usage: ./build-android-aarch64.sh
# Output: android/app/src/main/jniLibs/arm64-v8a/libclay.so

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$HOME/Android/Sdk/ndk/26.3.11579264}"
[[ -d "$ANDROID_NDK_HOME" ]] || {
    echo "error: NDK not found at $ANDROID_NDK_HOME (set ANDROID_NDK_HOME to override)" >&2
    exit 1
}

# API level matches android/app/build.gradle's minSdk (26).
API=26
TOOLCHAIN="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64"
CLANG="$TOOLCHAIN/bin/aarch64-linux-android${API}-clang"

[[ -x "$CLANG" ]] || {
    echo "error: NDK clang not found at $CLANG" >&2
    exit 1
}

# These are consumed by cargo/rustc (linker) and by cc-rs / ring's build scripts
# (rusqlite's bundled SQLite, ring's C/asm) for the aarch64-linux-android target.
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$CLANG"
export CC_aarch64_linux_android="$CLANG"
export CXX_aarch64_linux_android="${CLANG}++"
export AR_aarch64_linux_android="$TOOLCHAIN/bin/llvm-ar"

echo "Building clay for aarch64-linux-android (API $API)..."
cargo build --release --target aarch64-linux-android \
    --no-default-features --features rustls-backend,ssh-transport

BIN="target/aarch64-linux-android/release/clay"
OUT_DIR="android/app/src/main/jniLibs/arm64-v8a"
OUT="$OUT_DIR/libclay.so"

mkdir -p "$OUT_DIR"
cp "$BIN" "$OUT"
"$TOOLCHAIN/bin/llvm-strip" "$OUT"
echo "Done: $OUT"
file "$OUT"
