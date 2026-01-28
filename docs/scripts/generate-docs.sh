#!/bin/bash
# Main documentation generation script
# Combines screenshot generation and PDF/HTML creation

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DOCS_DIR="$(dirname "$SCRIPT_DIR")"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}"
echo "╔═══════════════════════════════════════════════════════════════╗"
echo "║           Clay Documentation Generation Script                 ║"
echo "╚═══════════════════════════════════════════════════════════════╝"
echo -e "${NC}"

# Parse command line arguments
SKIP_SCREENSHOTS=false
OUTPUT_FORMAT="pdf"
VERBOSE=false

while [[ $# -gt 0 ]]; do
    case $1 in
        --skip-screenshots|-s)
            SKIP_SCREENSHOTS=true
            shift
            ;;
        --html)
            OUTPUT_FORMAT="html"
            shift
            ;;
        --both)
            OUTPUT_FORMAT="both"
            shift
            ;;
        --verbose|-v)
            VERBOSE=true
            shift
            ;;
        --help|-h)
            echo "Usage: $0 [options]"
            echo ""
            echo "Options:"
            echo "  -s, --skip-screenshots  Skip screenshot generation"
            echo "  --html                  Generate HTML instead of PDF"
            echo "  --both                  Generate both PDF and HTML"
            echo "  -v, --verbose           Verbose output"
            echo "  -h, --help              Show this help message"
            exit 0
            ;;
        *)
            echo -e "${RED}Unknown option: $1${NC}"
            exit 1
            ;;
    esac
done

# Function to check dependencies
check_dependencies() {
    echo -e "${YELLOW}Checking dependencies...${NC}"
    local missing=()

    if ! command -v vhs &> /dev/null; then
        missing+=("vhs (go install github.com/charmbracelet/vhs@latest)")
    fi

    if ! command -v pandoc &> /dev/null; then
        missing+=("pandoc (sudo apt install pandoc)")
    fi

    if ! command -v xelatex &> /dev/null; then
        missing+=("xelatex (sudo apt install texlive-xetex)")
    fi

    if ! command -v convert &> /dev/null; then
        missing+=("ImageMagick (sudo apt install imagemagick)")
    fi

    if [ ${#missing[@]} -gt 0 ]; then
        echo -e "${RED}Missing dependencies:${NC}"
        for dep in "${missing[@]}"; do
            echo "  - $dep"
        done
        exit 1
    fi

    echo -e "${GREEN}All dependencies found.${NC}"
    echo ""
}

# Function to generate screenshots
generate_screenshots() {
    if [ "$SKIP_SCREENSHOTS" = true ]; then
        echo -e "${YELLOW}Skipping screenshot generation (--skip-screenshots)${NC}"
        return
    fi

    echo -e "${BLUE}=== Generating Screenshots ===${NC}"
    "$SCRIPT_DIR/generate-screenshots.sh"
    echo ""
}

# Function to process images
process_images() {
    echo -e "${BLUE}=== Processing Images ===${NC}"
    "$SCRIPT_DIR/process-images.sh"
    echo ""
}

# Function to combine markdown
combine_markdown() {
    echo -e "${BLUE}=== Combining Markdown Files ===${NC}"

    local output_dir="$DOCS_DIR/output"
    local markdown_dir="$DOCS_DIR/markdown"
    local combined="$output_dir/combined.md"

    mkdir -p "$output_dir"

    # Combine all markdown files in order
    cat "$markdown_dir"/*.md > "$combined"

    echo -e "${GREEN}Combined markdown created: $combined${NC}"
    echo ""
}

# Function to generate PDF
generate_pdf() {
    echo -e "${BLUE}=== Generating PDF ===${NC}"

    local output_dir="$DOCS_DIR/output"
    local combined="$output_dir/combined.md"
    local pdf_output="$output_dir/clay-documentation.pdf"

    pandoc "$combined" \
        -o "$pdf_output" \
        --toc \
        --toc-depth=3 \
        --pdf-engine=xelatex \
        --variable geometry:margin=1in \
        --variable fontsize=11pt \
        --variable documentclass=report \
        --variable colorlinks=true \
        --variable linkcolor=blue \
        --variable urlcolor=blue \
        --highlight-style=tango \
        --resource-path=".:$DOCS_DIR/images" \
        --metadata title="Clay MUD Client Documentation" \
        --metadata author="Clay Development Team" \
        --metadata date="$(date +%Y-%m-%d)"

    echo -e "${GREEN}PDF generated: $pdf_output${NC}"
    echo ""
}

# Function to generate HTML
generate_html() {
    echo -e "${BLUE}=== Generating HTML ===${NC}"

    local output_dir="$DOCS_DIR/output"
    local combined="$output_dir/combined.md"
    local html_output="$output_dir/clay-documentation.html"

    pandoc "$combined" \
        -o "$html_output" \
        --toc \
        --toc-depth=3 \
        --standalone \
        --highlight-style=tango \
        --resource-path=".:$DOCS_DIR/images" \
        --metadata title="Clay MUD Client Documentation"

    echo -e "${GREEN}HTML generated: $html_output${NC}"
    echo ""
}

# Main execution
check_dependencies
generate_screenshots
process_images
combine_markdown

case $OUTPUT_FORMAT in
    pdf)
        generate_pdf
        ;;
    html)
        generate_html
        ;;
    both)
        generate_pdf
        generate_html
        ;;
esac

echo -e "${GREEN}"
echo "╔═══════════════════════════════════════════════════════════════╗"
echo "║              Documentation Generation Complete!                ║"
echo "╚═══════════════════════════════════════════════════════════════╝"
echo -e "${NC}"

echo "Output files:"
ls -la "$DOCS_DIR/output/"*.pdf "$DOCS_DIR/output/"*.html 2>/dev/null || true
