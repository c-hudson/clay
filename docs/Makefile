# Clay Documentation Build System
# Generates PDF documentation with VHS terminal screenshots

.PHONY: all clean screenshots pdf html check-deps validate

# Directories
MARKDOWN_DIR = markdown
TAPES_DIR = tapes
IMAGES_DIR = images
OUTPUT_DIR = output
SCRIPTS_DIR = scripts

# Output files
PDF_OUTPUT = $(OUTPUT_DIR)/clay-documentation.pdf
HTML_OUTPUT = $(OUTPUT_DIR)/clay-documentation.html
COMBINED_MD = $(OUTPUT_DIR)/combined.md

# Find all source files
MARKDOWN_FILES = $(sort $(wildcard $(MARKDOWN_DIR)/*.md))
TAPE_FILES = $(wildcard $(TAPES_DIR)/*.tape)

# Default target
all: check-deps screenshots pdf

# Check for required dependencies
check-deps:
	@echo "Checking dependencies..."
	@which vhs > /dev/null || (echo "ERROR: vhs not found. Install with: go install github.com/charmbracelet/vhs@latest" && exit 1)
	@which pandoc > /dev/null || (echo "ERROR: pandoc not found. Install with: sudo apt install pandoc" && exit 1)
	@which xelatex > /dev/null || (echo "ERROR: xelatex not found. Install with: sudo apt install texlive-xetex" && exit 1)
	@which convert > /dev/null || (echo "ERROR: ImageMagick not found. Install with: sudo apt install imagemagick" && exit 1)
	@echo "All dependencies found."

# Validate VHS tape files
validate:
	@echo "Validating VHS tape files..."
	@for tape in $(TAPE_FILES); do \
		echo "  Validating $$tape..."; \
		vhs validate "$$tape" || exit 1; \
	done
	@echo "All tape files valid."

# Generate screenshots from VHS tapes
screenshots: $(TAPE_FILES)
	@echo "Generating screenshots..."
	@$(SCRIPTS_DIR)/generate-screenshots.sh

# Process images (convert GIFs to PNGs, optimize)
process-images:
	@echo "Processing images..."
	@$(SCRIPTS_DIR)/process-images.sh

# Combine markdown files
$(COMBINED_MD): $(MARKDOWN_FILES)
	@echo "Combining markdown files..."
	@mkdir -p $(OUTPUT_DIR)
	@cat $(MARKDOWN_FILES) > $(COMBINED_MD)
	@echo "Combined markdown created: $(COMBINED_MD)"

# Generate PDF
pdf: $(COMBINED_MD) process-images
	@echo "Generating PDF..."
	@pandoc $(COMBINED_MD) \
		-o $(PDF_OUTPUT) \
		--toc \
		--toc-depth=3 \
		--pdf-engine=xelatex \
		--variable geometry:margin=1in \
		--variable fontsize=11pt \
		--variable documentclass=report \
		--variable colorlinks=true \
		--variable linkcolor=blue \
		--variable urlcolor=blue \
		--variable mainfont="DejaVu Sans" \
		--variable sansfont="DejaVu Sans" \
		--variable monofont="DejaVu Sans Mono" \
		--highlight-style=tango \
		--resource-path=.:$(IMAGES_DIR) \
		--metadata title="Clay MUD Client Documentation" \
		--metadata author="Clay Development Team" \
		--metadata date="$(shell date +%Y-%m-%d)"
	@echo "PDF generated: $(PDF_OUTPUT)"

# Generate HTML (alternative output)
html: $(COMBINED_MD) process-images
	@echo "Generating HTML..."
	@pandoc $(COMBINED_MD) \
		-o $(HTML_OUTPUT) \
		--toc \
		--toc-depth=3 \
		--standalone \
		--highlight-style=tango \
		--resource-path=.:$(IMAGES_DIR) \
		--metadata title="Clay MUD Client Documentation"
	@echo "HTML generated: $(HTML_OUTPUT)"

# Generate screenshots without building PDF (useful for testing)
screenshots-only: check-deps validate
	@$(SCRIPTS_DIR)/generate-screenshots.sh

# Clean generated files
clean:
	rm -f $(COMBINED_MD)
	rm -f $(PDF_OUTPUT)
	rm -f $(HTML_OUTPUT)
	rm -f $(IMAGES_DIR)/tui/*.png
	rm -f $(IMAGES_DIR)/tui/*.gif
	rm -f $(IMAGES_DIR)/web/*.png
	rm -f $(IMAGES_DIR)/gui/*.png

# Clean everything including intermediate files
distclean: clean
	rm -rf $(OUTPUT_DIR)

# Help target
help:
	@echo "Clay Documentation Build System"
	@echo ""
	@echo "Targets:"
	@echo "  all          - Build everything (screenshots + PDF)"
	@echo "  screenshots  - Generate screenshots from VHS tapes"
	@echo "  pdf          - Generate PDF documentation"
	@echo "  html         - Generate HTML documentation"
	@echo "  validate     - Validate VHS tape files"
	@echo "  check-deps   - Check for required dependencies"
	@echo "  clean        - Remove generated files"
	@echo "  distclean    - Remove all generated files and directories"
	@echo "  help         - Show this help message"
	@echo ""
	@echo "Requirements:"
	@echo "  - vhs (go install github.com/charmbracelet/vhs@latest)"
	@echo "  - pandoc (sudo apt install pandoc)"
	@echo "  - xelatex (sudo apt install texlive-xetex)"
	@echo "  - ImageMagick (sudo apt install imagemagick)"
