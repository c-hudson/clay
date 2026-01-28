# Introduction

## What is Clay?

Clay is a modern terminal-based MUD (Multi-User Dungeon) client that combines the nostalgic feel of classic MUD clients with contemporary features and robust architecture. Built with Rust for performance and safety, Clay offers:

- **Multiple simultaneous world connections** - Connect to several MUDs at once
- **SSL/TLS support** - Secure connections to modern MUD servers
- **Rich ANSI color support** - Full 256-color and true color rendering
- **Web and GUI interfaces** - Access your MUD sessions from anywhere
- **TinyFugue compatibility** - Familiar scripting for TF veterans
- **Hot reload** - Update the client without losing connections

## Key Features

### Terminal User Interface (TUI)
- Clean, responsive interface built with ratatui/crossterm
- Unlimited scrollback buffer
- More-mode pausing for reading long output
- Spell checking with suggestions
- Command history and completion

### Multi-World Management
- Independent output buffers per world
- Activity indicators for background worlds
- Quick world switching with keyboard shortcuts
- Per-world settings (encoding, auto-login, logging)

### Automation & Scripting
- Actions/triggers with regex or wildcard patterns
- Capture group substitution in commands
- TinyFugue-compatible macro system
- Hooks for connect/disconnect events

### Remote Access
- WebSocket server for remote clients
- Browser-based web interface
- Native GUI client (egui)
- Remote console client

### Advanced Features
- Telnet protocol negotiation (SGA, TTYPE, NAWS, EOR)
- ANSI music playback
- Hot reload with connection preservation
- TLS proxy for SSL connection persistence
- File logging per world

## Supported Platforms

| Platform | Status | Notes |
|----------|--------|-------|
| Linux x86_64 | Full | Primary development platform |
| Linux ARM64 | Full | Tested on Raspberry Pi |
| macOS (Intel) | Full | Native builds |
| macOS (Apple Silicon) | Full | Native ARM64 builds |
| Windows (WSL) | Full | Via Windows Subsystem for Linux |
| Android (Termux) | Partial | Some features unavailable |

## Architecture Overview

Clay is built on an async architecture using Tokio:

    Main Event Loop
      |
      +-- Terminal Events (keyboard, resize)
      +-- WebSocket Server (remote clients)
      +-- World Tasks (one per connection)
      |
      v
    App State (central state manager)

Each connected world has:
- A reader task for incoming data
- A writer task for outgoing commands
- Independent output buffer and scroll state
- Per-world settings and connection state

\newpage

