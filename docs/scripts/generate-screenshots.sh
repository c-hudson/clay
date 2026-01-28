#!/bin/bash
# Generate screenshots from VHS tape files

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DOCS_DIR="$(dirname "$SCRIPT_DIR")"
TAPES_DIR="$DOCS_DIR/tapes"
IMAGES_DIR="$DOCS_DIR/images"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo "=== Generating Screenshots from VHS Tapes ==="
echo ""

# Check if vhs is installed
if ! command -v vhs &> /dev/null; then
    echo -e "${RED}ERROR: vhs is not installed${NC}"
    echo "Install with: go install github.com/charmbracelet/vhs@latest"
    exit 1
fi

# Create image directories if they don't exist
mkdir -p "$IMAGES_DIR/tui"
mkdir -p "$IMAGES_DIR/web"
mkdir -p "$IMAGES_DIR/gui"

# Process each tape file
for tape in "$TAPES_DIR"/*.tape; do
    if [ -f "$tape" ]; then
        tape_name=$(basename "$tape" .tape)
        echo -e "${YELLOW}Processing: $tape_name.tape${NC}"

        # Run vhs
        if vhs "$tape" 2>&1; then
            echo -e "${GREEN}  ✓ $tape_name completed${NC}"
        else
            echo -e "${RED}  ✗ $tape_name failed${NC}"
        fi
    fi
done

echo ""
echo "=== Screenshot Generation Complete ==="
echo ""

# List generated files
echo "Generated files:"
find "$IMAGES_DIR" -name "*.png" -o -name "*.gif" 2>/dev/null | sort | while read -r file; do
    echo "  $file"
done
