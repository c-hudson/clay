#!/bin/bash
# Apply Termux/Android X11 patches to winit, glutin, and glutin-winit
# These patches enable the remote GUI (egui) to run on Termux with Termux:X11
#
# Usage: ./patches/apply-patches.sh
#   Run from the clay project root directory BEFORE cargo fetch/build.
#   The script handles bootstrapping: creates stub dirs so cargo fetch works,
#   then replaces them with patched sources.

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
cd "$PROJECT_DIR"

# Crate name -> patched dir name -> package name (for stub Cargo.toml)
CRATE_NAMES=("winit-0.28.7" "glutin-0.30.10" "glutin-winit-0.3.0")
PATCHED_DIRS=("winit-0.28.7-patched" "glutin-0.30.10-patched" "glutin-winit-0.3.0-patched")
PKG_NAMES=("winit" "glutin" "glutin-winit")
PKG_VERSIONS=("0.28.7" "0.30.10" "0.3.0")

# Check if all patched dirs already have real content (not stubs)
all_done=true
for i in "${!CRATE_NAMES[@]}"; do
    dst="${PATCHED_DIRS[$i]}"
    if [ ! -d "$dst/src" ]; then
        all_done=false
        break
    fi
done

if $all_done; then
    echo "All patches already applied."
    exit 0
fi

# Step 1: Create stub Cargo.toml files so cargo fetch can resolve [patch.crates-io]
echo "Creating stub directories for cargo fetch..."
for i in "${!CRATE_NAMES[@]}"; do
    dst="${PATCHED_DIRS[$i]}"
    if [ -d "$dst/src" ]; then
        continue  # Already has real content
    fi
    mkdir -p "$dst/src"
    # Minimal stub so cargo can parse it
    cat > "$dst/Cargo.toml" << STUBEOF
[package]
name = "${PKG_NAMES[$i]}"
version = "${PKG_VERSIONS[$i]}"
edition = "2021"
STUBEOF
    # Stub lib.rs so it compiles (never actually built - replaced before build)
    echo "" > "$dst/src/lib.rs"
    echo "  ${CRATE_NAMES[$i]}: stub created"
done

# Step 2: Run cargo fetch to download real crate sources
echo "Running cargo fetch..."
cargo fetch

# Step 3: Find cargo registry
REGISTRY_BASE="$HOME/.cargo/registry/src"
REGISTRY_DIR=$(find "$REGISTRY_BASE" -maxdepth 1 -type d -name "index.crates.io-*" | head -1)
if [ -z "$REGISTRY_DIR" ]; then
    echo "Error: No crates.io index found in $REGISTRY_BASE after cargo fetch"
    exit 1
fi

# Step 4: Replace stubs with real sources + patches
for i in "${!CRATE_NAMES[@]}"; do
    crate="${CRATE_NAMES[$i]}"
    dst="${PATCHED_DIRS[$i]}"
    patch="$SCRIPT_DIR/$crate.patch"
    src="$REGISTRY_DIR/$crate"

    # Skip if already has real content (has more than just src/lib.rs)
    if [ -d "$dst/src" ] && [ "$(find "$dst/src" -name '*.rs' | wc -l)" -gt 1 ]; then
        echo "  $crate: already patched"
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

    echo "  $crate: done"
done

echo ""
echo "All patches applied. Ready to build with: cargo build --no-default-features --features rustls-backend,remote-gui"
