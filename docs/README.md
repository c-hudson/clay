# Clay Documentation

This directory contains the source files for Clay's PDF documentation.

## Directory Structure

```
docs/
├── markdown/           # Source documentation (21 chapters)
├── tapes/              # VHS tape files for terminal screenshots
├── images/             # Generated screenshots
│   ├── tui/           # Terminal interface screenshots
│   ├── web/           # Web interface screenshots
│   └── gui/           # GUI client screenshots
├── scripts/            # Build helper scripts
├── output/             # Generated PDF/HTML
├── Makefile           # Build automation
└── README.md          # This file
```

## Building the Documentation

### Prerequisites

Install required tools:

```bash
# VHS - Terminal recording/screenshots
go install github.com/charmbracelet/vhs@latest

# Pandoc + LaTeX for PDF generation
sudo apt install pandoc texlive-xetex texlive-latex-recommended

# ImageMagick for image processing
sudo apt install imagemagick

# Optional: pngquant for PNG optimization
sudo apt install pngquant

# Optional: Puppeteer for web screenshots
npm install -g puppeteer
```

### Quick Build

```bash
# Build everything (screenshots + PDF)
make all

# Or use the helper script
./scripts/generate-docs.sh
```

### Individual Targets

```bash
# Check dependencies
make check-deps

# Validate VHS tape files
make validate

# Generate screenshots only
make screenshots

# Process images (GIF to PNG, optimize)
make process-images

# Generate PDF only (assumes images exist)
make pdf

# Generate HTML instead
make html

# Clean generated files
make clean
```

### Build Options

```bash
# Skip screenshots (faster, uses existing images)
./scripts/generate-docs.sh --skip-screenshots

# Generate HTML instead of PDF
./scripts/generate-docs.sh --html

# Generate both PDF and HTML
./scripts/generate-docs.sh --both

# Verbose output
./scripts/generate-docs.sh --verbose
```

## Documentation Chapters

1. **Introduction** - Overview, features, architecture
2. **Installation** - Building from source, pre-built binaries
3. **Quick Start** - First launch, creating worlds, basic usage
4. **Interface Overview** - Screen layout, components, themes
5. **Commands** - All slash commands with options
6. **TinyFugue Commands** - TF compatibility layer
7. **Keyboard Shortcuts** - Complete shortcut reference
8. **Settings** - Global, per-world, and web settings
9. **Actions** - Triggers, patterns, capture groups
10. **Multi-World System** - Multiple connections, switching
11. **Web Interface** - Browser-based client
12. **Remote GUI Client** - Native graphical client
13. **Remote Console Client** - Terminal remote access
14. **Telnet Features** - Protocol support, prompts, keepalive
15. **ANSI Music** - Music sequence support
16. **Hot Reload** - Live binary updates
17. **TLS Proxy** - SSL connection preservation
18. **Spell Checking** - Dictionary-based spell check
19. **Android/Termux** - Mobile support
20. **Troubleshooting** - Common issues and solutions
21. **Appendices** - Reference tables, protocols, config format

## VHS Tape Files

VHS tapes define terminal recording/screenshot scenarios:

| Tape | Purpose |
|------|---------|
| startup.tape | Initial splash screen |
| popups.tape | World selector, editor, help |
| settings.tape | Setup and web settings |
| actions.tape | Actions list and editor |
| more-mode.tape | More-mode pausing |
| filter.tape | Filter popup |
| commands.tape | Command demonstrations |
| tf-commands.tape | TinyFugue commands |
| spell-check.tape | Spell checking demo |
| world-switching.tape | Multi-world navigation |
| connect-world.tape | Connection process |

## Adding New Screenshots

1. Create a new tape file in `tapes/`:
   ```tape
   Output docs/images/tui/my-screenshot.gif
   Set Shell "bash"
   Set FontSize 14
   Set Width 100
   Set Height 30
   Type "./clay"
   Enter
   Sleep 2s
   Screenshot docs/images/tui/my-screenshot.png
   ```

2. Run `make screenshots` to generate

3. Reference in markdown: `![Description](images/tui/my-screenshot.png)`

## Web Interface Screenshots

Use the capture script with Puppeteer:

```bash
# Start Clay with web interface enabled
./clay  # Configure /web settings first

# Run capture script
node scripts/capture-web.js http://localhost:9000 docs/images/web
```

Set `CLAY_WEB_PASSWORD` environment variable to authenticate.

## Output

Generated files are placed in `output/`:

- `clay-documentation.pdf` - Full PDF documentation
- `clay-documentation.html` - HTML version (if generated)
- `combined.md` - Intermediate combined markdown

## Customization

### PDF Styling

Edit Makefile variables for PDF customization:

```makefile
--variable geometry:margin=1in
--variable fontsize=11pt
--variable documentclass=report
```

### Adding Chapters

1. Create new markdown file in `markdown/` with numeric prefix (e.g., `21-new-chapter.md`)
2. Files are combined in alphabetical order
3. Run `make pdf` to rebuild

## Troubleshooting

### VHS Not Found

```bash
go install github.com/charmbracelet/vhs@latest
export PATH=$PATH:$(go env GOPATH)/bin
```

### LaTeX Errors

```bash
sudo apt install texlive-latex-extra texlive-fonts-recommended
```

### Image Not Found in PDF

Ensure images are in `images/` directory and paths in markdown match.
