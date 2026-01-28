# ANSI Music

Clay supports ANSI music sequences, a feature from BBS-era computing that allows servers to send simple melodies.

## What is ANSI Music?

ANSI music uses escape sequences to define melodies that play on the client's speaker. The format originated with PC BBS software in the 1990s and uses a syntax similar to the BASIC PLAY command.

## Format

ANSI music sequences have this structure:

```
ESC [ M <music_string> Ctrl-N
```

Or with modifiers:

```
ESC [ MF <music_string> Ctrl-N    # Foreground (blocks output)
ESC [ MB <music_string> Ctrl-N    # Background (concurrent)
ESC [ N <music_string> Ctrl-N     # Alternative format
```

Where:
- `ESC` is the escape character (0x1B)
- `Ctrl-N` (0x0E) marks the end of the sequence

## Music String Syntax

The music string uses BASIC PLAY command notation:

### Notes

| Command | Description |
|---------|-------------|
| A-G | Play note A through G |
| # or + | Sharp (follows note) |
| - | Flat (follows note) |
| . | Dotted note (1.5x duration) |
| P or R | Pause/rest |

### Octave

| Command | Description |
|---------|-------------|
| O\<n\> | Set octave (0-6, default 4) |
| \> | Increase octave |
| \< | Decrease octave |

### Tempo and Duration

| Command | Description |
|---------|-------------|
| T\<n\> | Set tempo (32-255 BPM, default 120) |
| L\<n\> | Set default note length (1=whole, 4=quarter, etc.) |

### Example

```
T120 L4 O4 CDEFGAB>C
```

Plays C major scale at 120 BPM, quarter notes, starting at octave 4.

## Configuration

### Enable ANSI Music

1. Open `/setup`
2. Enable "ANSI Music"

### Test Audio

Use the test command to verify audio works:

```
/testmusic
```

This plays a simple C-D-E-F-G sequence.

## Playback

### Console

The console itself cannot play audio (no speaker access from terminal). Music sequences are:
- Extracted from output
- Stripped from display
- Forwarded to connected web/GUI clients

### Web Interface

Uses Web Audio API:
- Square wave oscillator (PC speaker simulation)
- Plays through browser audio
- May require user interaction to start (browser autoplay policy)

### Remote GUI

Requires `remote-gui-audio` feature:

```bash
# Build with audio support
sudo apt install libasound2-dev  # Linux only
cargo build --features remote-gui-audio
```

Uses rodio library with:
- ALSA backend on Linux
- CoreAudio on macOS

## Building with Audio

### Linux

```bash
# Install ALSA development libraries
sudo apt install libasound2-dev

# Build with audio
cargo build --features remote-gui-audio
```

### macOS

No extra dependencies needed:

```bash
cargo build --features remote-gui-audio
```

Audio uses CoreAudio automatically.

## Troubleshooting

### No Sound in Web Interface

1. Check browser allows audio (click somewhere first)
2. Verify ANSI Music is enabled in `/setup`
3. Check browser console for errors
4. Try different browser

### No Sound in GUI

1. Verify built with `remote-gui-audio` feature
2. Check ANSI Music is enabled in `/setup`
3. Verify ALSA is working: `aplay /usr/share/sounds/alsa/Front_Center.wav`
4. Check PulseAudio/PipeWire is running

### Music Sounds Wrong

1. ANSI music is limited to PC speaker frequencies
2. Complex sequences may not sound as intended
3. Timing may vary based on system load

### Music Not Playing

1. Verify MUD actually sends ANSI music sequences
2. Check sequences are properly formatted
3. Use `/testmusic` to verify client audio works

## Technical Details

### Sequence Detection

Clay detects ANSI music by looking for:
1. ESC `[` followed by `M` or `N`
2. Optional modifier (F or B)
3. Music string content
4. Terminating Ctrl-N (0x0E)

### Processing

1. Music sequences extracted during output processing
2. Sequences forwarded to web/GUI clients via WebSocket
3. Display shows output with music sequences removed

### Web Audio Implementation

```javascript
// Simplified example
const oscillator = audioContext.createOscillator();
oscillator.type = 'square';
oscillator.frequency.setValueAtTime(frequency, time);
oscillator.start(time);
oscillator.stop(time + duration);
```

### Frequency Calculation

Note frequencies calculated from:
- Base: A4 = 440 Hz
- Formula: `freq = 440 * 2^((note-A4)/12)`

\newpage

