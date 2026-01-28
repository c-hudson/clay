#!/bin/bash
# Process images: convert GIFs to PNGs, optimize for documentation

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DOCS_DIR="$(dirname "$SCRIPT_DIR")"
IMAGES_DIR="$DOCS_DIR/images"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo "=== Processing Images ==="
echo ""

# Check if ImageMagick is installed
if ! command -v convert &> /dev/null; then
    echo -e "${RED}ERROR: ImageMagick is not installed${NC}"
    echo "Install with: sudo apt install imagemagick"
    exit 1
fi

# Enable globstar for ** pattern matching
shopt -s globstar nullglob

# Convert GIFs to PNGs (extract first frame) - but don't overwrite existing PNGs
echo "Converting GIFs to PNGs..."
gif_count=0
for gif in "$IMAGES_DIR"/**/*.gif "$IMAGES_DIR"/*.gif; do
    if [ -f "$gif" ]; then
        png="${gif%.gif}.png"

        # Skip if PNG already exists and is larger than 10KB (likely a screenshot)
        if [ -f "$png" ]; then
            size=$(stat -c%s "$png" 2>/dev/null || stat -f%z "$png" 2>/dev/null)
            if [ "$size" -gt 10000 ]; then
                echo -e "${YELLOW}  Skipping: $(basename "$gif") (screenshot exists)${NC}"
                continue
            fi
        fi

        echo -e "${YELLOW}  Converting: $(basename "$gif")${NC}"

        # Extract first frame and convert to PNG
        if convert "${gif}[0]" "$png" 2>/dev/null; then
            echo -e "${GREEN}    ✓ Created: $(basename "$png")${NC}"
            ((gif_count++)) || true
        else
            echo -e "${RED}    ✗ Failed to convert: $(basename "$gif")${NC}"
        fi
    fi
done

if [ $gif_count -eq 0 ]; then
    echo "  No GIF files found to convert."
fi

# Optimize PNGs (reduce file size while maintaining quality)
echo ""
echo "Optimizing PNGs..."
png_count=0
for png in "$IMAGES_DIR"/**/*.png "$IMAGES_DIR"/*.png; do
    if [ -f "$png" ]; then
        original_size=$(stat -c%s "$png" 2>/dev/null || stat -f%z "$png" 2>/dev/null)

        # Use pngquant if available, otherwise skip optimization
        if command -v pngquant &> /dev/null; then
            pngquant --force --quality=80-95 --output "$png" "$png" 2>/dev/null || true
            new_size=$(stat -c%s "$png" 2>/dev/null || stat -f%z "$png" 2>/dev/null)
            echo -e "${GREEN}  ✓ Optimized: $(basename "$png") ($original_size -> $new_size bytes)${NC}"
            ((png_count++)) || true
        fi
    fi
done

if [ $png_count -eq 0 ]; then
    if ! command -v pngquant &> /dev/null; then
        echo "  pngquant not installed, skipping optimization."
    else
        echo "  No PNG files found to optimize."
    fi
fi

echo ""
echo "=== Image Processing Complete ==="
