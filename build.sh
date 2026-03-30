#!/bin/bash

# Build script for Clay MUD Client
# Automatically detects available features and falls back gracefully

set -e

REQUIRED_RUST_VERSION="1.75.0"

echo "=============================================="
echo "  Clay MUD Client - Build Script"
echo "=============================================="
echo ""

# Detect OS and environment
OS="unknown"
IS_TERMUX=false
IS_ANDROID=false

if [ -n "$TERMUX_VERSION" ] || [ -d "/data/data/com.termux" ]; then
    OS="termux"
    IS_TERMUX=true
    IS_ANDROID=true
elif [[ "$OSTYPE" == "linux-gnu"* ]]; then
    OS="linux"
elif [[ "$OSTYPE" == "linux-android"* ]]; then
    OS="android"
    IS_ANDROID=true
elif [[ "$OSTYPE" == "darwin"* ]]; then
    OS="macos"
elif [[ "$OSTYPE" == "msys"* ]] || [[ "$OSTYPE" == "cygwin"* ]] || [[ "$OSTYPE" == "win32"* ]]; then
    OS="windows"
elif [[ "$OSTYPE" == "freebsd"* ]]; then
    OS="freebsd"
fi

echo "Platform:         $OS"

# Check if cargo is installed
if ! command -v cargo &> /dev/null; then
    echo ""
    echo "Error: Cargo is not installed."
    echo ""
    echo "Install Rust via:"
    echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    exit 1
fi

# Check Rust version
RUST_VERSION=$(rustc --version | grep -oE '[0-9]+\.[0-9]+\.[0-9]+')
echo "Rust version:     $RUST_VERSION (minimum: $REQUIRED_RUST_VERSION)"

version_ge() {
    [ "$(printf '%s\n' "$1" "$2" | sort -V | head -n1)" = "$2" ]
}

if ! version_ge "$RUST_VERSION" "$REQUIRED_RUST_VERSION"; then
    echo ""
    echo "Error: Rust version $REQUIRED_RUST_VERSION or higher is required."
    echo "Update Rust via: rustup update"
    exit 1
fi

# Detect available features
FEATURES=""
HAS_WEBVIEW=false
HAS_AUDIO=false
HAS_MUSL=false
EXTRA_RUSTFLAGS=""
MISSING_DEPS=""
BUILD_TARGET=""

echo ""
echo "Detecting available features..."

# --- TLS backend ---
# Always use rustls (works everywhere, no OpenSSL dependency)
FEATURES="rustls-backend"
echo "  [+] rustls-backend (TLS)"

# --- musl static linking (Linux only) ---
if [ "$OS" = "linux" ]; then
    MUSL_TARGET="x86_64-unknown-linux-musl"
    if rustup target list --installed 2>/dev/null | grep -q "$MUSL_TARGET"; then
        HAS_MUSL=true
        echo "  [+] musl target available ($MUSL_TARGET)"
    else
        echo "  [-] musl target not installed (will use native target)"
        echo "      Install with: rustup target add $MUSL_TARGET"
    fi
fi

# --- WebView GUI ---
check_webview() {
    case "$OS" in
        linux)
            # Need webkit2gtk and gtk3
            if command -v pkg-config &> /dev/null; then
                if pkg-config --exists webkit2gtk-4.1 2>/dev/null && \
                   pkg-config --exists gtk+-3.0 2>/dev/null; then
                    return 0
                fi
                # Check what's missing for helpful error
                if ! pkg-config --exists webkit2gtk-4.1 2>/dev/null; then
                    MISSING_DEPS="$MISSING_DEPS libwebkit2gtk-4.1-dev"
                fi
                if ! pkg-config --exists gtk+-3.0 2>/dev/null; then
                    MISSING_DEPS="$MISSING_DEPS libgtk-3-dev"
                fi
            else
                MISSING_DEPS="$MISSING_DEPS pkg-config"
            fi
            return 1
            ;;
        macos|windows)
            # WebView is always available (WKWebView / WebView2)
            return 0
            ;;
        termux)
            # Need patched tao/wry + Termux:X11
            if [ -f "tao-0.34.5-patched/.patched" ] && [ -f "wry-0.48.1-patched/.patched" ]; then
                return 0
            elif [ -f "patches/apply-patches.sh" ]; then
                echo "  [?] WebView patches not applied, applying now..."
                if ./patches/apply-patches.sh; then
                    return 0
                fi
            fi
            return 1
            ;;
        *)
            return 1
            ;;
    esac
}

if check_webview; then
    HAS_WEBVIEW=true
    FEATURES="$FEATURES,webview-gui"
    echo "  [+] webview-gui (native GUI)"
else
    echo "  [-] webview-gui (not available)"
    if [ -n "$MISSING_DEPS" ]; then
        echo "      Missing:$MISSING_DEPS"
        case "$OS" in
            linux)
                echo "      Install: sudo apt install$MISSING_DEPS"
                ;;
        esac
    fi
fi

# --- Native audio ---
check_audio() {
    if $IS_ANDROID; then
        # oboe/NDK C++ deps not available on Termux
        return 1
    fi
    case "$OS" in
        linux)
            if command -v pkg-config &> /dev/null && pkg-config --exists alsa 2>/dev/null; then
                return 0
            fi
            MISSING_DEPS="$MISSING_DEPS libasound2-dev"
            return 1
            ;;
        macos|windows)
            # CoreAudio / WASAPI always available
            return 0
            ;;
        *)
            return 1
            ;;
    esac
}

if check_audio; then
    HAS_AUDIO=true
    FEATURES="$FEATURES,native-audio"
    echo "  [+] native-audio (sound playback)"
else
    echo "  [-] native-audio (not available)"
fi

# --- Termux-specific setup ---
if $IS_TERMUX; then
    # Need -L /system/lib64 for libandroid.so (ndk crate)
    if [ -f "/system/lib64/libandroid.so" ]; then
        EXTRA_RUSTFLAGS="-L /system/lib64"
        echo "  [+] Android system libraries found"
    elif [ -f "/system/lib/libandroid.so" ]; then
        EXTRA_RUSTFLAGS="-L /system/lib"
        echo "  [+] Android system libraries found (32-bit)"
    fi
fi

# --- Determine build target ---
if [ "$OS" = "linux" ] && $HAS_MUSL && ! $HAS_WEBVIEW && ! $HAS_AUDIO; then
    # Static musl build (TUI only, no GUI/audio)
    BUILD_TARGET="--target x86_64-unknown-linux-musl --no-default-features"
else
    # Native target
    BUILD_TARGET="--no-default-features"
fi

# --- Windows static CRT ---
if [ "$OS" = "windows" ]; then
    EXTRA_RUSTFLAGS="-C target-feature=+crt-static"
    echo "  [+] Static CRT linking (no vcruntime140.dll dependency)"
fi

echo ""
echo "Features: $FEATURES"
echo ""
echo "Building release..."
echo ""

# --- Build with fallback ---
build_with_fallback() {
    local features="$1"
    local target_flags="$2"
    local rustflags="$3"
    local desc="$4"

    echo "Attempting: $desc"
    if RUSTFLAGS="$rustflags" cargo build --release $target_flags --features "$features" 2>&1; then
        return 0
    fi
    return 1
}

BUILD_OK=false

# Try full feature set first
if build_with_fallback "$FEATURES" "$BUILD_TARGET" "$EXTRA_RUSTFLAGS" "full feature build ($FEATURES)"; then
    BUILD_OK=true
fi

# Fallback: try without audio
if ! $BUILD_OK && $HAS_AUDIO; then
    echo ""
    echo "Full build failed. Retrying without native-audio..."
    FALLBACK_FEATURES=$(echo "$FEATURES" | sed 's/,native-audio//')
    if build_with_fallback "$FALLBACK_FEATURES" "$BUILD_TARGET" "$EXTRA_RUSTFLAGS" "without audio ($FALLBACK_FEATURES)"; then
        BUILD_OK=true
        FEATURES="$FALLBACK_FEATURES"
    fi
fi

# Fallback: try without webview
if ! $BUILD_OK && $HAS_WEBVIEW; then
    echo ""
    echo "Build failed. Retrying without webview-gui..."
    FALLBACK_FEATURES=$(echo "$FEATURES" | sed 's/,webview-gui//' | sed 's/,native-audio//')
    if build_with_fallback "$FALLBACK_FEATURES" "$BUILD_TARGET" "$EXTRA_RUSTFLAGS" "TUI only ($FALLBACK_FEATURES)"; then
        BUILD_OK=true
        FEATURES="$FALLBACK_FEATURES"
    fi
fi

# Fallback: try musl static on Linux
if ! $BUILD_OK && [ "$OS" = "linux" ] && $HAS_MUSL; then
    echo ""
    echo "Build failed. Retrying with musl static build..."
    if build_with_fallback "rustls-backend" "--target x86_64-unknown-linux-musl --no-default-features" "" "musl static (rustls-backend only)"; then
        BUILD_OK=true
        FEATURES="rustls-backend"
        BUILD_TARGET="--target x86_64-unknown-linux-musl --no-default-features"
    fi
fi

# Final fallback: minimal build
if ! $BUILD_OK; then
    echo ""
    echo "All attempts failed. Trying minimal build..."
    if build_with_fallback "rustls-backend" "--no-default-features" "" "minimal (rustls-backend only)"; then
        BUILD_OK=true
        FEATURES="rustls-backend"
    fi
fi

if ! $BUILD_OK; then
    echo ""
    echo "=============================================="
    echo "  Build failed!"
    echo "=============================================="
    echo ""
    echo "Troubleshooting:"
    echo "  1. Make sure Rust is up to date: rustup update"
    echo "  2. Check for missing system libraries above"
    echo "  3. Report issues at: https://github.com/c-hudson/clay/issues"
    exit 1
fi

# Find the built binary
if echo "$BUILD_TARGET" | grep -q "musl"; then
    BINARY_PATH="target/x86_64-unknown-linux-musl/release/clay"
else
    BINARY_PATH="target/release/clay"
fi

if [ ! -f "$BINARY_PATH" ]; then
    # Try native target path
    NATIVE_TARGET=$(rustc -vV | grep host | cut -d' ' -f2)
    BINARY_PATH="target/$NATIVE_TARGET/release/clay"
fi

if [ ! -f "$BINARY_PATH" ]; then
    echo "Error: Could not find built binary"
    exit 1
fi

SIZE=$(du -h "$BINARY_PATH" | cut -f1)

echo ""
echo "=============================================="
echo "  Build successful!"
echo "=============================================="
echo ""
echo "Binary:   $BINARY_PATH ($SIZE)"
echo "Features: $FEATURES"
echo ""

# Prompt for install location
DEFAULT_DEST="$HOME"
read -p "Install location [$DEFAULT_DEST]: " DEST

if [ -z "$DEST" ]; then
    DEST="$DEFAULT_DEST"
fi

# Expand ~ if used
DEST="${DEST/#\~/$HOME}"

if [ -d "$DEST" ]; then
    DEST="$DEST/clay"
fi

echo ""
echo "Installing to $DEST..."
cp "$BINARY_PATH" "$DEST"
chmod +x "$DEST"

echo ""
echo "=============================================="
echo "  Installation complete!"
echo "=============================================="
echo ""
echo "Binary: $DEST"
echo ""
echo "Run Clay:"
echo "  $DEST"
if echo "$FEATURES" | grep -q "webview-gui"; then
    echo ""
    echo "Run as GUI client (connects to running instance):"
    echo "  $DEST --gui=hostname:port"
fi
echo ""
