#!/bin/bash
# Apply Termux/Android X11 patches to winit, glutin, and glutin-winit
# These patches enable the remote GUI (egui) to run on Termux with Termux:X11
#
# Usage: ./patches/apply-patches.sh
#   Run from the clay project root directory.
#   Requires: cargo fetch to have been run first (crates in ~/.cargo/registry)

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

# Find cargo registry source directory
REGISTRY_BASE="$HOME/.cargo/registry/src"
if [ ! -d "$REGISTRY_BASE" ]; then
    echo "Error: Cargo registry not found at $REGISTRY_BASE"
    echo "Run 'cargo fetch' first to download dependencies."
    exit 1
fi

# Find the registry index directory (name varies by system)
REGISTRY_DIR=$(find "$REGISTRY_BASE" -maxdepth 1 -type d -name "index.crates.io-*" | head -1)
if [ -z "$REGISTRY_DIR" ]; then
    echo "Error: No crates.io index found in $REGISTRY_BASE"
    echo "Run 'cargo fetch' first to download dependencies."
    exit 1
fi

# Crates to patch
declare -A CRATES=(
    ["winit-0.28.7"]="winit-0.28.7-patched"
    ["glutin-0.30.10"]="glutin-0.30.10-patched"
    ["glutin-winit-0.3.0"]="glutin-winit-0.3.0-patched"
)

for crate in "${!CRATES[@]}"; do
    src="$REGISTRY_DIR/$crate"
    dst="$PROJECT_DIR/${CRATES[$crate]}"
    patch="$SCRIPT_DIR/$crate.patch"

    if [ ! -d "$src" ]; then
        echo "Error: Source crate not found at $src"
        echo "Run 'cargo fetch' to download it."
        exit 1
    fi

    if [ ! -f "$patch" ]; then
        echo "Error: Patch file not found at $patch"
        exit 1
    fi

    # Check if already patched
    if [ -d "$dst" ]; then
        echo "  $crate: already patched (${CRATES[$crate]}/ exists)"
        continue
    fi

    echo "  $crate: copying from cargo registry..."
    cp -r "$src" "$dst"

    echo "  $crate: applying patch..."
    cd "$dst"
    patch -p1 --no-backup-if-mismatch < "$patch"
    cd "$PROJECT_DIR"

    echo "  $crate: done"
done

echo ""
echo "All patches applied. The following directories are ready:"
for crate in "${!CRATES[@]}"; do
    echo "  ${CRATES[$crate]}/"
done
echo ""
echo "These are referenced by [patch.crates-io] in Cargo.toml."
