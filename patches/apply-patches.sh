#!/bin/bash
# Apply Termux/Android X11 patches to winit, glutin, glutin-winit, tao, and wry
# These patches enable the remote GUI (egui) and webview GUI (wry) to run on Termux with Termux:X11
#
# Usage: ./patches/apply-patches.sh
#   Run from the clay project root directory BEFORE cargo build.
#   The script handles bootstrapping: creates stub dirs so cargo fetch works,
#   then replaces them with patched sources.

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
cd "$PROJECT_DIR"

CRATE_NAMES=("winit-0.28.7" "glutin-0.30.10" "glutin-winit-0.3.0" "tao-0.34.5" "wry-0.48.1")
PATCHED_DIRS=("winit-0.28.7-patched" "glutin-0.30.10-patched" "glutin-winit-0.3.0-patched" "tao-0.34.5-patched" "wry-0.48.1-patched")

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

# Step 1: Create stub Cargo.toml files so cargo fetch can resolve [patch.crates-io]
# Stubs must declare all features that downstream crates reference.
echo "Creating stub directories for cargo fetch..."

needs_fetch=false

# winit stub
dst="${PATCHED_DIRS[0]}"
if ! is_patched "$dst"; then
    needs_fetch=true
    rm -rf "$dst"
    mkdir -p "$dst/src"
    cat > "$dst/Cargo.toml" << 'EOF'
[package]
name = "winit"
version = "0.28.7"
edition = "2021"

[features]
default = ["x11", "wayland", "wayland-dlopen", "wayland-csd-adwaita"]
x11 = []
wayland = []
wayland-dlopen = []
wayland-csd-adwaita = []
wayland-csd-adwaita-crossfont = []
wayland-csd-adwaita-notitle = []
android-game-activity = []
android-native-activity = []
serde = []
mint = []

[dependencies]
raw-window-handle = { version = "0.5", package = "raw-window-handle" }
EOF
    echo "" > "$dst/src/lib.rs"
    echo "  winit-0.28.7: stub created"
fi

# glutin stub
dst="${PATCHED_DIRS[1]}"
if ! is_patched "$dst"; then
    needs_fetch=true
    rm -rf "$dst"
    mkdir -p "$dst/src"
    cat > "$dst/Cargo.toml" << 'EOF'
[package]
name = "glutin"
version = "0.30.10"
edition = "2021"

[features]
default = ["egl", "glx", "x11", "wayland", "wgl"]
egl = []
glx = ["x11"]
x11 = []
wayland = []
wgl = []

[dependencies]
raw-window-handle = "0.5"
EOF
    echo "" > "$dst/src/lib.rs"
    echo "  glutin-0.30.10: stub created"
fi

# glutin-winit stub
dst="${PATCHED_DIRS[2]}"
if ! is_patched "$dst"; then
    needs_fetch=true
    rm -rf "$dst"
    mkdir -p "$dst/src"
    cat > "$dst/Cargo.toml" << 'EOF'
[package]
name = "glutin-winit"
version = "0.3.0"
edition = "2021"

[features]
default = ["egl", "glx", "x11", "wayland", "wgl"]
egl = []
glx = ["x11"]
x11 = []
wayland = []
wgl = []

[dependencies]
raw-window-handle = "0.5"
EOF
    echo "" > "$dst/src/lib.rs"
    echo "  glutin-winit-0.3.0: stub created"
fi

if ! $needs_fetch; then
    echo "All patches already applied."
    exit 0
fi

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

echo ""
echo "All patches applied. Ready to build with: cargo build --no-default-features --features rustls-backend,remote-gui"
