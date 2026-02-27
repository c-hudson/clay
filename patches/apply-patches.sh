#!/bin/bash
# Apply Termux/Android X11 patches to tao and wry
# These patches enable the webview GUI (wry) to run on Termux with Termux:X11
#
# Usage: ./patches/apply-patches.sh
#   Run from the clay project root directory BEFORE cargo build.
#   The script handles bootstrapping: creates stub dirs so cargo fetch works,
#   then replaces them with patched sources.

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
cd "$PROJECT_DIR"

CRATE_NAMES=("tao-0.34.5" "wry-0.48.1")
PATCHED_DIRS=("tao-0.34.5-patched" "wry-0.48.1-patched")

# Check if all patched dirs are fully patched.
# Uses a .patched marker file to avoid false positives when git pull
# restores stub Cargo.toml over real source files.
is_patched() {
    local dir="$1"
    [ -f "$dir/.patched" ] && [ -d "$dir/src" ] && [ "$(find "$dir/src" -name '*.rs' | wc -l)" -gt 1 ]
}

all_done=true
for i in "${!CRATE_NAMES[@]}"; do
    if ! is_patched "${PATCHED_DIRS[$i]}"; then
        all_done=false
        break
    fi
done

if $all_done; then
    echo "All patches already applied."
    exit 0
fi

# Step 1: Run cargo fetch to download real crate sources
needs_fetch=false
for i in "${!CRATE_NAMES[@]}"; do
    if ! is_patched "${PATCHED_DIRS[$i]}"; then
        needs_fetch=true
        break
    fi
done

if $needs_fetch; then
    echo "Running cargo fetch..."
    cargo fetch
fi

# Step 2: Find cargo registry
REGISTRY_BASE="$HOME/.cargo/registry/src"
REGISTRY_DIR=$(find "$REGISTRY_BASE" -maxdepth 1 -type d -name "index.crates.io-*" | head -1)
if [ -z "$REGISTRY_DIR" ]; then
    echo "Error: No crates.io index found in $REGISTRY_BASE after cargo fetch"
    exit 1
fi

# Step 3: Replace stubs with real sources + patches
for i in "${!CRATE_NAMES[@]}"; do
    crate="${CRATE_NAMES[$i]}"
    dst="${PATCHED_DIRS[$i]}"
    patch="$SCRIPT_DIR/$crate.patch"
    src="$REGISTRY_DIR/$crate"

    if is_patched "$dst"; then
        echo "  $crate: already patched, skipping"
        continue
    fi

    if [ ! -d "$src" ]; then
        echo "Error: Source crate not found at $src"
        exit 1
    fi

    if [ ! -f "$patch" ]; then
        echo "Error: Patch file not found at $patch"
        exit 1
    fi

    echo "  $crate: copying from cargo registry..."
    rm -rf "$dst"
    cp -r "$src" "$dst"

    echo "  $crate: applying patch..."
    cd "$dst"
    patch -p1 --no-backup-if-mismatch < "$patch"
    cd "$PROJECT_DIR"

    # Write marker so git pull doesn't fool the check
    touch "$dst/.patched"

    echo "  $crate: done"
done

# Step 4: Add [patch.crates-io] to Cargo.toml if not already present
if ! grep -q '\[patch.crates-io\]' Cargo.toml; then
    echo ""
    echo "Adding [patch.crates-io] to Cargo.toml..."
    cat >> Cargo.toml <<'PATCH'

[patch.crates-io]
tao = { path = "tao-0.34.5-patched" }
wry = { path = "wry-0.48.1-patched" }
PATCH
fi

echo ""
echo "All patches applied. Ready to build with: cargo build --no-default-features --features rustls-backend,webview-gui"
