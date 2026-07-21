#!/usr/bin/env bash
# Cross-compile the Termux no-GUI (TUI) binary for 64-bit ARM (aarch64 / arm64-v8a)
# using the Android NDK. Mirrors build-termux-armv7.sh's approach: Termux doesn't ship
# its own libc — it links against Android's Bionic (/system/lib64/libc.so), the exact
# libc the NDK's aarch64-linux-android<API>-clang targets, so a straight NDK cross-compile
# is ABI-compatible with the on-device Termux aarch64 build this replaces (see
# .claude/skills/release/machines.md). The only Termux-specific step is the patchelf
# rpath fixup below, since a Termux binary runs outside the Android app's linker
# namespace (unlike build-android-aarch64.sh's output, which runs as an ordinary app
# subprocess and needs no rpath patching at all).
#
# The GUI variant (webview-gui: wry/tao/webkit2gtk/X11) is NOT built here — it requires
# linking against Termux's own compiled GTK3/WebKit2GTK/X11 libraries at
# /data/data/com.termux/files/usr/lib, which only exist inside the Termux userland (see
# patches/apply-patches.sh). That one still has to build on-device; see machines.md's
# "Termux aarch64 binary (with GUI, no audio)" section.
#
# One-time setup (see .claude/skills/release/machines.md for details):
#   - Android NDK r26d unpacked at $ANDROID_NDK_HOME (default: ~/Android/Sdk/ndk/26.3.11579264)
#   - rustup target add aarch64-linux-android
#   - patchelf on PATH (installed to ~/.local/bin here; no root required)
#
# Usage: ./build-termux-aarch64.sh
# Output: /tmp/clay-termux-aarch64-nogui

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$HOME/Android/Sdk/ndk/26.3.11579264}"
[[ -d "$ANDROID_NDK_HOME" ]] || {
    echo "error: NDK not found at $ANDROID_NDK_HOME (set ANDROID_NDK_HOME to override)" >&2
    exit 1
}

# API level for the generated clang wrapper. 24 matches build-termux-armv7.sh — no app
# minSdk to match here (unlike build-android-aarch64.sh's 26), so keep both Termux
# cross-compiles on the same, already-proven-on-this-phone API level.
API=24
TOOLCHAIN="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64"
CLANG="$TOOLCHAIN/bin/aarch64-linux-android${API}-clang"

[[ -x "$CLANG" ]] || {
    echo "error: NDK clang not found at $CLANG" >&2
    exit 1
}

command -v patchelf >/dev/null || {
    echo "error: patchelf not found on PATH" >&2
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
OUT="/tmp/clay-termux-aarch64-nogui"

# Same rpath the on-device Termux build applies: Bionic system libs plus Termux's own
# userland libs, since this binary still runs inside Termux regardless of where it was
# compiled.
patchelf --set-rpath '/system/lib64:/data/data/com.termux/files/usr/lib' "$BIN"

cp "$BIN" "$OUT"
echo "Done: $OUT"
file "$OUT"
