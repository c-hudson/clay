#!/bin/bash
# Build both console (musl) and GUI versions of Clay
# Console: target/x86_64-unknown-linux-musl/debug/clay
# GUI:     target/debug/clay

set -e

echo "Building console version (musl)..."
cargo build --target x86_64-unknown-linux-musl --no-default-features --features rustls-backend

echo ""
echo "Building GUI version (remote-gui + audio)..."
cargo build --features remote-gui,rodio

echo ""
echo "Build complete!"
echo "  Console: target/x86_64-unknown-linux-musl/debug/clay"
echo "  GUI:     target/debug/clay"
