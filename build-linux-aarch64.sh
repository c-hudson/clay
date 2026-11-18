#!/usr/bin/env bash
# Cross-compile the Linux ARM64 release asset: a static musl build (console, -D, the web
# server; no GUI or audio), as clay-linux-x86_64-musl is for x86_64. It runs on any
# ARM64 Linux: Raspberry Pi 4/5 on 64-bit Pi OS, ARM servers (Oracle, Hetzner,
# AWS Graviton), Asahi Linux.
#
# The C code (ring, rusqlite's bundled SQLite) is compiled by Zig, which carries musl
# and its headers for every target, as build-drobo5n.sh does for ARMv7. Zig is fetched
# on first use into ~/.cache/clay-zig (SHA256 pinned below) and never installed.
#
# Usage: ./build-linux-aarch64.sh
# Output: /tmp/clay-linux-aarch64-musl

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

ZIG_VERSION=0.17.1
ZIG_SHA256=3581863e3cbcfd0fce0efc682ce7e9e93da96668a977791d356a26d2a7e9b17a
ZIG_NAME="zig-x86_64-linux-$ZIG_VERSION"
ZIG_HOME="$HOME/.cache/clay-zig"
ZIG="$ZIG_HOME/$ZIG_NAME/zig"
TRIPLE=aarch64-unknown-linux-musl
OUT=/tmp/clay-linux-aarch64-musl

if [[ ! -x "$ZIG" ]]; then
    mkdir -p "$ZIG_HOME"
    tarball="$ZIG_HOME/$ZIG_NAME.tar.xz"
    echo "Fetching Zig $ZIG_VERSION..."
    curl -fsSL -o "$tarball.part" "https://ziglang.org/download/$ZIG_VERSION/$ZIG_NAME.tar.xz"
    echo "$ZIG_SHA256  $tarball.part" | sha256sum -c --quiet - || {
        rm -f "$tarball.part"
        echo "error: SHA256 mismatch for $ZIG_NAME.tar.xz" >&2
        exit 1
    }
    tar -xJf "$tarball.part" -C "$ZIG_HOME"
    rm -f "$tarball.part"
fi

rustup target list --installed | grep -qx "$TRIPLE" || rustup target add "$TRIPLE"

# The cc crate and rustc want one executable per tool, not `zig cc -target ...`. cc-rs
# takes zig for clang and adds a --target= with Rust's triple, which zig can't parse.
WRAP=target/zig-aarch64
mkdir -p "$WRAP"
for tool in cc c++; do
    cat > "$WRAP/$tool" <<EOF
#!/usr/bin/env bash
args=()
for a in "\$@"; do [[ \$a == --target=* ]] || args+=("\$a"); done
exec "$ZIG" $tool -target aarch64-linux-musl "\${args[@]}"
EOF
    chmod +x "$WRAP/$tool"
done
printf '#!/bin/sh\nexec "%s" ar "$@"\n' "$ZIG" > "$WRAP/ar"
chmod +x "$WRAP/ar"

export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER="$PWD/$WRAP/cc"
export CC_aarch64_unknown_linux_musl="$PWD/$WRAP/cc"
export CXX_aarch64_unknown_linux_musl="$PWD/$WRAP/c++"
export AR_aarch64_unknown_linux_musl="$PWD/$WRAP/ar"
# Zig supplies musl's CRT, libc and libunwind itself; rustc's own copies of them as well
# give "duplicate symbol: _start" (see build-drobo5n.sh).
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_RUSTFLAGS="-C link-self-contained=no"

echo "Building clay for $TRIPLE..."
cargo build --release --target "$TRIPLE" --no-default-features --features ssh-transport,shell-worlds

cp "target/$TRIPLE/release/clay" "$OUT"
echo "Done: $OUT"
file "$OUT" 2>/dev/null || true
