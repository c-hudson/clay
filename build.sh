#!/bin/bash

# Build script for Clay MUD Client with GUI and audio support

set -e

echo "Building Clay MUD Client (release, GUI + audio)..."
echo ""

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
