#!/bin/bash

# Build script for Clay MUD Client with GUI and audio support

set -e

REQUIRED_RUST_VERSION="1.75.0"

echo "=============================================="
echo "  Clay MUD Client - Build Script"
echo "=============================================="
echo ""

# Detect OS
OS="unknown"
if [[ "$OSTYPE" == "linux-gnu"* ]]; then
    OS="linux"
elif [[ "$OSTYPE" == "darwin"* ]]; then
    OS="macos"
fi

# Check if cargo is installed
if ! command -v cargo &> /dev/null; then
    echo "Error: Cargo is not installed."
    echo ""
    echo "Install Rust via:"
    echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    exit 1
fi

# Check Rust version
RUST_VERSION=$(rustc --version | grep -oE '[0-9]+\.[0-9]+\.[0-9]+')
echo "Rust version:     $RUST_VERSION (minimum: $REQUIRED_RUST_VERSION)"

# Compare versions
version_ge() {
    [ "$(printf '%s\n' "$1" "$2" | sort -V | head -n1)" = "$2" ]
}

if ! version_ge "$RUST_VERSION" "$REQUIRED_RUST_VERSION"; then
    echo ""
    echo "Error: Rust version $REQUIRED_RUST_VERSION or higher is required."
    echo ""
    echo "Update Rust via:"
    echo "  rustup update"
    echo ""
    echo "Or reinstall:"
    echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    exit 1
fi

# Check for system dependencies on Linux
MISSING_DEPS=""
BUILD_FEATURES="remote-gui-audio"

if [ "$OS" = "linux" ]; then
    echo ""
    echo "Checking system dependencies..."

    # Check for pkg-config
    if ! command -v pkg-config &> /dev/null; then
        MISSING_DEPS="$MISSING_DEPS pkg-config"
    fi

    # Check for ALSA development libraries (needed for audio)
    if ! pkg-config --exists alsa 2>/dev/null; then
        MISSING_DEPS="$MISSING_DEPS libasound2-dev"
    fi

    # Check for X11/xcb libraries (needed for GUI)
    if ! pkg-config --exists xcb 2>/dev/null; then
        MISSING_DEPS="$MISSING_DEPS libxcb-dev"
    fi

    if ! pkg-config --exists xcb-render 2>/dev/null; then
        MISSING_DEPS="$MISSING_DEPS libxcb-render0-dev"
    fi

    if ! pkg-config --exists xcb-shape 2>/dev/null; then
        MISSING_DEPS="$MISSING_DEPS libxcb-shape0-dev"
    fi

    if ! pkg-config --exists xcb-xfixes 2>/dev/null; then
        MISSING_DEPS="$MISSING_DEPS libxcb-xfixes0-dev"
    fi

    if [ -n "$MISSING_DEPS" ]; then
        echo ""
        echo "Warning: Missing system dependencies:$MISSING_DEPS"
        echo ""
        echo "Install on Debian/Ubuntu:"
        echo "  sudo apt install$MISSING_DEPS"
        echo ""
        echo "Install on Fedora:"
        echo "  sudo dnf install alsa-lib-devel libxcb-devel"
        echo ""
        echo "Install on Arch:"
        echo "  sudo pacman -S alsa-lib libxcb"
        echo ""
        read -p "Continue anyway? [y/N]: " CONTINUE
        if [[ ! "$CONTINUE" =~ ^[Yy]$ ]]; then
            echo ""
            echo "You can also build without GUI/audio:"
            echo "  cargo build --release"
            exit 1
        fi
    else
        echo "All dependencies found."
    fi
fi

echo ""

# Remove Cargo.lock to avoid version compatibility issues
if [ -f "Cargo.lock" ]; then
    echo "Removing Cargo.lock for compatibility..."
    rm Cargo.lock
fi

echo ""
echo "Building with features: $BUILD_FEATURES"
echo "This may take a few minutes..."
echo ""

# Build release with remote-gui-audio feature
if ! cargo build --release --features "$BUILD_FEATURES"; then
    echo ""
    echo "=============================================="
    echo "  Build failed!"
    echo "=============================================="
    echo ""
    echo "Troubleshooting:"
    echo "  1. Make sure all system dependencies are installed"
    echo "  2. Try updating Rust: rustup update"
    echo "  3. Try building without GUI/audio: cargo build --release"
    exit 1
fi

echo ""
echo "=============================================="
echo "  Build successful!"
echo "=============================================="
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

echo ""
echo "=============================================="
echo "  Installation complete!"
echo "=============================================="
echo ""
echo "Binary installed to: $DEST"
echo ""
echo "Run the TUI client:"
echo "  $DEST"
echo ""
echo "Run as remote GUI client:"
echo "  $DEST --remote=hostname:port"
echo ""
