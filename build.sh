#!/bin/bash

# Build script for Clay MUD Client with GUI and audio support

set -e

REQUIRED_RUST_VERSION="1.75.0"

echo "Building Clay MUD Client (release, GUI + audio)..."
echo ""

# Check if cargo is installed
if ! command -v cargo &> /dev/null; then
    echo "Error: Cargo is not installed."
    echo "Install Rust via: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    exit 1
fi

# Check Rust version
RUST_VERSION=$(rustc --version | grep -oE '[0-9]+\.[0-9]+\.[0-9]+')
echo "Found Rust version: $RUST_VERSION"
echo "Required minimum:   $REQUIRED_RUST_VERSION"

# Compare versions (simple numeric comparison)
version_ge() {
    [ "$(printf '%s\n' "$1" "$2" | sort -V | head -n1)" = "$2" ]
}

if ! version_ge "$RUST_VERSION" "$REQUIRED_RUST_VERSION"; then
    echo ""
    echo "Error: Rust version $REQUIRED_RUST_VERSION or higher is required."
    echo "Update Rust via: rustup update"
    echo "Or reinstall:    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    exit 1
fi

echo ""

# Remove Cargo.lock to avoid version compatibility issues
if [ -f "Cargo.lock" ]; then
    echo "Removing Cargo.lock for compatibility..."
    rm Cargo.lock
fi

# Build release with remote-gui-audio feature
cargo build --release --features remote-gui-audio

if [ $? -ne 0 ]; then
    echo "Build failed!"
    exit 1
fi

echo ""
echo "Build successful!"
echo ""

# Prompt for install location
DEFAULT_DEST="$HOME"
read -p "Install location [$DEFAULT_DEST]: " DEST

# Use default if empty
if [ -z "$DEST" ]; then
    DEST="$DEFAULT_DEST"
fi

# Expand ~ if used
DEST="${DEST/#\~/$HOME}"

# Check if destination is a directory
if [ -d "$DEST" ]; then
    DEST="$DEST/clay"
fi

# Copy the binary
echo ""
echo "Copying to $DEST..."
cp target/release/clay "$DEST"
chmod +x "$DEST"

echo "Done! Installed to: $DEST"
