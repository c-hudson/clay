#!/usr/bin/env bash
# Build the Linux GUI release asset (native GUI + audio, glibc, x86_64) so that it runs
# on older distributions, not only on one as new as this machine.
#
# A binary needs the newest glibc symbol version it was linked against. Linked here it
# needed glibc 2.43 (v1.7.38: acosf/atan2f/... got new versions in 2.43), which only the
# newest distributions have. This links against Ubuntu 20.04's glibc 2.31 and its
# X11/ALSA libraries instead (tools/glibc-sysroot.py), so the asset runs on Ubuntu
# 20.04+, Debian 11+, Mint 20+, Fedora 32+ and RHEL 9+. The compiler and Rust are this
# machine's own; only what is linked against comes from the sysroot.
#
# Checks before it says Done: the binary needs no glibc symbol version above $FLOOR,
# and it starts (`--version`) under the sysroot's own 2.31 loader and libraries with
# LD_BIND_NOW=1, which binds every symbol from every library it links at start.
#
# Usage: ./build-linux-gui.sh
# Output: /tmp/clay-linux-x86_64-gui

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

FLOOR=2.31
TRIPLE=x86_64-unknown-linux-gnu
TOOLCHAIN=${NATIVE_GUI_TOOLCHAIN:-1.98.1}
OUT=/tmp/clay-linux-x86_64-gui

SYSROOT=$(python3 -I tools/glibc-sysroot.py | tail -1)
[[ -e "$SYSROOT/usr/lib/x86_64-linux-gnu/libc.so" ]] || {
    echo "error: no sysroot at $SYSROOT" >&2
    exit 1
}

# pkg-config (alsa-sys) reads only the sysroot's .pc files and prefixes their paths.
export PKG_CONFIG_SYSROOT_DIR="$SYSROOT"
export PKG_CONFIG_LIBDIR="$SYSROOT/usr/lib/x86_64-linux-gnu/pkgconfig:$SYSROOT/usr/share/pkgconfig"
unset PKG_CONFIG_PATH
# C and C++ code (ring, the bundled SQLite, freetype-sys's own FreeType) is compiled
# against the sysroot's headers, and the final link searches the sysroot. gcc also
# searches its own library directory, this machine's, so the sysroot's library
# directories are named first: -lgcc_s and -lstdc++ would otherwise find this machine's
# copies, which need glibc 2.34 and 2.38. With an explicit --target these RUSTFLAGS reach
# only the target's artifacts: build scripts and proc-macros, which run here, link
# against this machine's glibc.
export CFLAGS_x86_64_unknown_linux_gnu="--sysroot=$SYSROOT"
export CXXFLAGS_x86_64_unknown_linux_gnu="--sysroot=$SYSROOT"
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS="-C link-arg=--sysroot=$SYSROOT -L native=$SYSROOT/lib/x86_64-linux-gnu -L native=$SYSROOT/usr/lib/x86_64-linux-gnu"

echo "Building the Linux GUI asset against glibc $FLOOR ($SYSROOT)..."
# target/native-gui is cargo-native.sh's target dir; with --target the build lands in its
# own x86_64-unknown-linux-gnu/ subdirectory and shares only the host build scripts.
cargo "+$TOOLCHAIN" build --release --target "$TRIPLE" \
    --features native-audio,native-gui --target-dir target/native-gui

BIN="target/native-gui/$TRIPLE/release/clay"

need=$(objdump -T "$BIN" | grep -o 'GLIBC_[0-9.]*' | sed 's/GLIBC_//' | sort -uV | tail -1)
if [[ "$(printf '%s\n%s\n' "$need" "$FLOOR" | sort -V | tail -1)" != "$FLOOR" ]]; then
    echo "error: $BIN needs glibc $need, above $FLOOR:" >&2
    objdump -T "$BIN" | grep -E "GLIBC_$need\b" >&2
    exit 1
fi

LIBS="$SYSROOT/lib/x86_64-linux-gnu:$SYSROOT/usr/lib/x86_64-linux-gnu"
if ! version=$(LD_BIND_NOW=1 "$SYSROOT/lib64/ld-linux-x86-64.so.2" --library-path "$LIBS" \
        "$BIN" --version 2>&1); then
    echo "error: $BIN does not start against glibc $FLOOR:" >&2
    echo "$version" >&2
    exit 1
fi

cp "$BIN" "$OUT"
echo "Needs glibc $need (floor $FLOOR); starts under the $FLOOR loader:"
echo "$version" | head -2
echo "Done: $OUT"
