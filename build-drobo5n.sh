#!/usr/bin/env bash
# Cross-compile a static clay binary for the Drobo 5N NAS (armv7l, Marvell PJ4Bv7 /
# Armada XP, kernel 3.2.96). Runs on the drobo@192.168.2.48 build VM, which was
# originally set up to cross-compile C/C++ for the 5N with a Marvell gcc 4.4.5
# toolchain at ~/xtools/toolchain/5n/bin/. That toolchain's sysroot is eglibc 2.11.1
# (2010), too old for Rust's std (needs glibc >= 2.17), so it is not used here.
# Instead this targets armv7-unknown-linux-musleabi (fully static, zero dependency
# on the Drobo's userland) using Zig as the C cross-compiler for `ring` and
# rusqlite's bundled SQLite - see the wrapper scripts below.
#
# One-time setup on the VM:
#   - rustup (i686 host) with `rustup target add armv7-unknown-linux-musleabi`
#   - Zig 0.16.0 (x86-linux build) unpacked at ~/zig
#     (https://ziglang.org/download/0.16.0/zig-x86-linux-0.16.0.tar.xz)
#   - ~/bin/arm-musl-cc, ~/bin/arm-musl-cxx, ~/bin/arm-musl-ar wrapping
#     `zig cc`/`zig c++`/`zig ar -target arm-linux-musleabi` (the cc crate needs a
#     plain executable, not a multi-word command)
#
# Usage: ./build-drobo5n.sh
# Output: /tmp/clay-drobo5n-armv7

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

ZIG_CC="$HOME/bin/arm-musl-cc"
ZIG_CXX="$HOME/bin/arm-musl-cxx"
ZIG_AR="$HOME/bin/arm-musl-ar"

for tool in "$ZIG_CC" "$ZIG_CXX" "$ZIG_AR"; do
    [[ -x "$tool" ]] || {
        echo "error: $tool not found (see one-time setup in this script's header)" >&2
        exit 1
    }
done

command -v cargo >/dev/null || {
    echo "error: cargo not found (install rustup: https://static.rust-lang.org/rustup/dist/i686-unknown-linux-gnu/rustup-init)" >&2
    exit 1
}

# These are consumed by cargo/rustc (linker) and by cc-rs / ring's build scripts
# (rusqlite's bundled SQLite, ring's C/asm) for the armv7-unknown-linux-musleabi
# target. Same pattern as build-termux-armv7.sh's NDK clang wiring.
export CARGO_TARGET_ARMV7_UNKNOWN_LINUX_MUSLEABI_LINKER="$ZIG_CC"
export CC_armv7_unknown_linux_musleabi="$ZIG_CC"
export CXX_armv7_unknown_linux_musleabi="$ZIG_CXX"
export AR_armv7_unknown_linux_musleabi="$ZIG_AR"

# On this VM /usr/bin/cc and /usr/bin/gcc are symlinked to the Marvell ARM
# cross-gcc (it was set up for C/C++ cross-compiles, not native host builds).
# Proc-macro/build-script crates (proc-macro2, etc.) still compile and link for
# the HOST (i686-unknown-linux-gnu), and rustc's default host linker is `cc` -
# without an override that silently resolves to the ARM gcc and produces
# "Relocations in generic ELF (EM: 3)" link failures. Pin the host linker and
# host cc-rs builds to the real native compiler instead.
HOST_GCC="/usr/bin/gcc-4.8"
[[ -x "$HOST_GCC" ]] || {
    echo "error: native host compiler not found at $HOST_GCC" >&2
    exit 1
}
export CARGO_TARGET_I686_UNKNOWN_LINUX_GNU_LINKER="$HOST_GCC"
export CC_i686_unknown_linux_gnu="$HOST_GCC"
export CXX_i686_unknown_linux_gnu="/usr/bin/g++-4.8"
export HOST_CC="$HOST_GCC"

# Rust's musl targets bundle their own CRT/libc/unwind objects ("self contained"
# mode) by default. Zig's musl target unconditionally supplies its own too, and
# passing -nostartfiles/-nodefaultlibs doesn't stop it - the result is a
# "duplicate symbol: _start" link error from mixing the two CRTs. The granular
# per-component flag (`-C link-self-contained=-crto`) needs nightly
# (-Z unstable-options); the coarse stable `no` disables all of rustc's
# self-contained pieces and cedes CRT/libc/unwind entirely to Zig's own musl
# build, which is complete and self-consistent (this is also the standard fix
# for the same issue in cargo-zigbuild).
export RUSTFLAGS="-C link-self-contained=no"

echo "Building clay for armv7-unknown-linux-musleabi (Drobo 5N)..."
cargo build --release --target armv7-unknown-linux-musleabi \
    --no-default-features --features rustls-backend,ssh-transport

BIN="target/armv7-unknown-linux-musleabi/release/clay"
OUT="/tmp/clay-drobo5n-armv7"

cp "$BIN" "$OUT"
echo "Done: $OUT"
file "$OUT" 2>/dev/null || true
