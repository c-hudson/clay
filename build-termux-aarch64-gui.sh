#!/usr/bin/env bash
# Cross-compile the Termux GUI binary (webview-gui: wry/tao/WebKit2GTK/GTK3/X11) for
# aarch64 with the Android NDK - no phone needed.
#
# Termux builds every package it ships by cross-compiling with the NDK, so its own
# aarch64 .debs are a ready-made sysroot: tools/termux-sysroot.py downloads the
# dependency closure of webkit2gtk-4.1/gtk3/libc++ and unpacks it. pkg-config is then
# pointed at that sysroot, and the link uses the same NDK clang as
# build-termux-aarch64.sh (Termux links against Android's Bionic, which is exactly what
# the NDK targets).
#
# The tao/wry patches (patches/apply-patches.sh) append [patch.crates-io] to Cargo.toml
# and create *-patched source dirs, so the build runs in a throwaway git worktree at HEAD
# - the main checkout is never modified.
#
# One-time setup: the same as build-termux-aarch64.sh (NDK r26d, `rustup target add
# aarch64-linux-android`, patchelf on PATH), plus dpkg-deb (for the sysroot).
#
# Usage: ./build-termux-aarch64-gui.sh
# Output: /tmp/clay-termux-aarch64

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$HOME/Android/Sdk/ndk/26.3.11579264}"
[[ -d "$ANDROID_NDK_HOME" ]] || {
    echo "error: NDK not found at $ANDROID_NDK_HOME (set ANDROID_NDK_HOME to override)" >&2
    exit 1
}
API=24
TOOLCHAIN="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64"
CLANG="$TOOLCHAIN/bin/aarch64-linux-android${API}-clang"
[[ -x "$CLANG" ]] || { echo "error: NDK clang not found at $CLANG" >&2; exit 1; }
command -v patchelf >/dev/null || { echo "error: patchelf not found on PATH" >&2; exit 1; }

SYSROOT="$(tools/termux-sysroot.py | tail -1)"
PREFIX="$SYSROOT/data/data/com.termux/files/usr"
[[ -f "$PREFIX/lib/pkgconfig/webkit2gtk-4.1.pc" ]] || {
    echo "error: sysroot at $SYSROOT has no webkit2gtk-4.1" >&2
    exit 1
}

WT="$SCRIPT_DIR/target/termux-gui-wt"
cleanup() { git -C "$SCRIPT_DIR" worktree remove --force "$WT" 2>/dev/null || true; }
cleanup
git worktree add --detach "$WT" HEAD >/dev/null
trap cleanup EXIT

cd "$WT"
./patches/apply-patches.sh >/dev/null

export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$CLANG"
export CC_aarch64_linux_android="$CLANG"
export CXX_aarch64_linux_android="${CLANG}++"
export AR_aarch64_linux_android="$TOOLCHAIN/bin/llvm-ar"
# Termux's .pc files carry the absolute on-device prefix; SYSROOT_DIR re-roots them.
export PKG_CONFIG_ALLOW_CROSS=1
export PKG_CONFIG_SYSROOT_DIR="$SYSROOT"
export PKG_CONFIG_LIBDIR="$PREFIX/lib/pkgconfig:$PREFIX/share/pkgconfig"
unset PKG_CONFIG_PATH
# -rpath-link lets the linker resolve the GTK/WebKit libraries' own DT_NEEDED entries.
export CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS="-L $PREFIX/lib -C link-arg=-Wl,-rpath-link,$PREFIX/lib"
# Shared with the main checkout's target/: all the dependency builds are reused
# between releases. Cargo holds its lock only while this build runs.
export CARGO_TARGET_DIR="$SCRIPT_DIR/target/termux-gui"

echo "Building clay (webview GUI) for aarch64-linux-android (API $API)..."
cargo build --release --target aarch64-linux-android \
    --no-default-features --features rustls-backend,webview-gui,ssh-transport

BIN="$CARGO_TARGET_DIR/aarch64-linux-android/release/clay"
OUT="/tmp/clay-termux-aarch64"
# The same rpath the on-device build applied: Bionic plus Termux's userland libs.
patchelf --set-rpath '/system/lib64:/data/data/com.termux/files/usr/lib' "$BIN"
cp "$BIN" "$OUT"
echo "Done: $OUT"
file "$OUT"
