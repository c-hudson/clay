//! ANSI Music Parser
//!
//! Parses ANSI music sequences that start with ESC [ and end with Ctrl-N (0x0E).
//! The content between uses BASIC PLAY command syntax.

use serde::{Deserialize, Serialize};

/// A single note to be played
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MusicNote {
    /// Frequency in Hz (0 = rest/silence)
    pub frequency: f32,
    /// Duration in milliseconds
    pub duration_ms: u32,
}

/// Result of parsing ANSI music from a data stream
pub struct ParseResult {
    /// Data before the music sequence (to be displayed)
    pub before: String,
    /// The parsed music notes
    pub notes: Vec<MusicNote>,
    /// Data after the music sequence (to be processed further)
    pub after: String,
}

/// Parser state for ANSI music sequences
#[derive(Default)]
pub struct AnsiMusicParser {
    /// Current octave (0-6, default 4)
    octave: u8,
    /// Current note length (1=whole, 2=half, 4=quarter, etc.)
    length: u8,
    /// Tempo in quarter notes per minute (default 120)
    tempo: u16,
    /// Note style: 0=normal (7/8), 1=legato (full), 2=staccato (3/4)
    style: u8,
}

/// Characters that can legitimately appear in an ANSI-music (MML) payload: notes,
/// octave/tempo/length/pause commands, the `M`-prefixed modifiers, durations and
/// separators. Used only by `find_sequence`'s unterminated-sequence fallback, to tell a
/// genuine cut-off tune from ordinary text following an `ESC[M` (Delete Lines).
fn is_mml_char(c: char) -> bool {
    matches!(c,
        'A'..='G' | 'a'..='g'          // notes
        | 'O' | 'o' | 'T' | 't'        // octave, tempo
        | 'L' | 'l' | 'P' | 'p'        // length, pause
        | 'M' | 'm' | 'N' | 'n'        // modifiers (MF/MB/ML/MN/MS), note-by-number
        | 'S' | 's' | 'X' | 'x'        // style, execute
        // note: the F/B modifier letters are already covered by 'A'..='G' above
        | '0'..='9'
        | '.' | '#' | '+' | '-' | '<' | '>' | ' ' | ';' | ','
    )
}

impl AnsiMusicParser {
    pub fn new() -> Self {
        Self {
            octave: 4,
            length: 4,  // Quarter note default
            tempo: 120,
            style: 0,   // Normal
        }
    }

    /// Reset parser state to defaults
    pub fn reset(&mut self) {
        self.octave = 4;
        self.length = 4;
        self.tempo = 120;
        self.style = 0;
    }

    /// Locate one ANSI music sequence, or `None` if there isn't one.
    ///
    /// Supported formats:
    ///
    /// - `ESC [ M ... Ctrl-N` (standard ANSI music)
    /// - `ESC [ MF ... Ctrl-N` (foreground music)
    /// - `ESC [ MB ... Ctrl-N` (background music)
    /// - `ESC [ N ... Ctrl-N` (alternate format)
    ///
    /// Returns `(start, content_end, consumed_end)`:
    ///
    /// - `start` — index of the `ESC`
    /// - `content_end` — end of the music payload, i.e. what to hand to `parse`
    /// - `consumed_end` — end of what to remove from the text stream
    ///
    /// The two differ because a `Ctrl-N` terminator is consumed while a newline one is
    /// left in the stream. The caller used to guess at this with `end - 1`, which both
    /// dropped the last note of a newline-terminated sequence and panicked whenever
    /// `end - 1` landed inside a multi-byte character. Returning the payload boundary
    /// explicitly removes the guess.
    ///
    /// Every returned index is a char boundary: they are all positions of ASCII bytes
    /// (`ESC`, `Ctrl-N`, `\n`, `\r`) or the end of the string.
    pub fn find_sequence(data: &str) -> Option<(usize, usize, usize)> {
        let bytes = data.as_bytes();
        let mut i = 0;

        while i < bytes.len().saturating_sub(2) {
            // Look for ESC [
            if bytes[i] == 0x1B && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
                let start = i;
                let mut j = i + 2;

                // Check for M or N prefix
                if j < bytes.len() && (bytes[j] == b'M' || bytes[j] == b'N') {
                    j += 1;

                    // Skip optional F (foreground) or B (background) modifier
                    if j < bytes.len() && (bytes[j] == b'F' || bytes[j] == b'B') {
                        j += 1;
                    }

                    // Look for Ctrl-N (0x0E) terminator
                    while j < bytes.len() {
                        if bytes[j] == 0x0E {
                            // Ctrl-N terminates and is itself consumed.
                            return Some((start, j, j + 1));
                        }
                        j += 1;
                    }

                    // No Ctrl-N found - check for newline as alternate terminator
                    // Some MUDs terminate with newline instead
                    j = i + 2;
                    if j < bytes.len() && (bytes[j] == b'M' || bytes[j] == b'N') {
                        j += 1;
                        if j < bytes.len() && (bytes[j] == b'F' || bytes[j] == b'B') {
                            j += 1;
                        }
                        while j < bytes.len() {
                            if bytes[j] == b'\n' || bytes[j] == b'\r' {
                                // The newline terminates but stays in the text stream.
                                return Some((start, j, j));
                            }
                            j += 1;
                        }
                        // No terminator at all. The chunk may have been cut mid-sequence,
                        // so the rest can still be music — but `ESC[M` is also an ordinary
                        // CSI (Delete Lines), and this branch decides whether to DELETE the
                        // remainder of the packet from the display.
                        //
                        // This used to accept the rest if *any* character in it was one of
                        // T/L/O/A-G/a-g, which is true of essentially all English prose, so
                        // a single `ESC[M` silently ate everything after it. Require the
                        // whole remainder to be plausible MML instead: a false negative
                        // merely means a tune does not play, while a false positive
                        // destroys the user's text.
                        let rest = &data[i + 2..];
                        if !rest.is_empty() && rest.chars().all(is_mml_char) {
                            return Some((start, bytes.len(), bytes.len()));
                        }
                    }
                }
            }
            i += 1;
        }
        None
    }

    /// Parse an ANSI music sequence and return the notes
    /// Input should be the content between ESC [ and Ctrl-N
    pub fn parse(&mut self, sequence: &str) -> Vec<MusicNote> {
        let mut notes = Vec::new();
        let chars: Vec<char> = sequence.chars().collect();
        let mut i = 0;

        // Skip leading 'M' or 'N' if present
        if i < chars.len() && (chars[i] == 'M' || chars[i] == 'N') {
            i += 1;
        }

        while i < chars.len() {
            let c = chars[i].to_ascii_uppercase();

            match c {
                // Notes A-G
                'A'..='G' => {
                    let note_value = self.note_to_semitone(c);
                    i += 1;

                    // Check for sharp (#, +) or flat (-)
                    let modifier = if i < chars.len() {
                        match chars[i] {
                            '#' | '+' => { i += 1; 1i8 }
                            '-' => { i += 1; -1i8 }
                            _ => 0i8
                        }
                    } else {
                        0i8
                    };

                    // Check for optional length suffix
                    let note_length = self.parse_number(&chars, &mut i).unwrap_or(self.length as u32) as u8;

                    // Count dots for dotted notes
                    let mut dots = 0;
                    while i < chars.len() && chars[i] == '.' {
                        dots += 1;
                        i += 1;
                    }

                    let freq = self.calculate_frequency(note_value, modifier);
                    let duration = self.calculate_duration(note_length, dots);

                    notes.push(MusicNote {
                        frequency: freq,
                        duration_ms: duration,
                    });
                }

                // Note by number (N0-N84)
                'N' if i + 1 < chars.len() => {
                    i += 1;
                    if let Some(num) = self.parse_number(&chars, &mut i) {
                        if num == 0 {
                            // N0 is a rest
                            notes.push(MusicNote {
                                frequency: 0.0,
                                duration_ms: self.calculate_duration(self.length, 0),
                            });
                        } else if num <= 84 {
                            let freq = self.note_number_to_frequency(num as u8);
                            notes.push(MusicNote {
                                frequency: freq,
                                duration_ms: self.calculate_duration(self.length, 0),
                            });
                        }
                    }
                }

                // Octave (O0-O6)
                'O' => {
                    i += 1;
                    if let Some(oct) = self.parse_number(&chars, &mut i) {
                        if oct <= 6 {
                            self.octave = oct as u8;
                        }
                    }
                }

                // Length (L1-L64)
                'L' => {
                    i += 1;
                    if let Some(len) = self.parse_number(&chars, &mut i) {
                        if (1..=64).contains(&len) {
                            self.length = len as u8;
                        }
                    }
                }

                // Pause/Rest (P1-P64)
                'P' => {
                    i += 1;
                    let rest_length = self.parse_number(&chars, &mut i).unwrap_or(self.length as u32) as u8;

                    // Count dots
                    let mut dots = 0;
                    while i < chars.len() && chars[i] == '.' {
                        dots += 1;
                        i += 1;
                    }

                    notes.push(MusicNote {
                        frequency: 0.0,
                        duration_ms: self.calculate_duration(rest_length, dots),
                    });
                }

                // Tempo (T32-T255)
                'T' => {
                    i += 1;
                    if let Some(tempo) = self.parse_number(&chars, &mut i) {
                        if (32..=255).contains(&tempo) {
                            self.tempo = tempo as u16;
                        }
                    }
                }

                // Music style commands
                'M' => {
                    i += 1;
                    if i < chars.len() {
                        match chars[i].to_ascii_uppercase() {
                            'N' => { self.style = 0; i += 1; } // Normal
                            'L' => { self.style = 1; i += 1; } // Legato
                            'S' => { self.style = 2; i += 1; } // Staccato
                            'F' | 'B' => { i += 1; } // Foreground/Background - ignore
                            _ => {}
                        }
                    }
                }

                // Octave up
                '>' => {
                    if self.octave < 6 {
                        self.octave += 1;
                    }
                    i += 1;
                }

                // Octave down
                '<' => {
                    if self.octave > 0 {
                        self.octave -= 1;
                    }
                    i += 1;
                }

                // Skip whitespace and unknown characters
                _ => {
                    i += 1;
                }
            }
        }

        notes
    }

    /// Parse a number from the character stream
    fn parse_number(&self, chars: &[char], i: &mut usize) -> Option<u32> {
        let start = *i;
        while *i < chars.len() && chars[*i].is_ascii_digit() {
            *i += 1;
        }
        if *i > start {
            let s: String = chars[start..*i].iter().collect();
            s.parse().ok()
        } else {
            None
        }
    }

    /// Convert note letter to semitone offset within octave (C=0, D=2, E=4, F=5, G=7, A=9, B=11)
    fn note_to_semitone(&self, note: char) -> i8 {
        match note {
            'C' => 0,
            'D' => 2,
            'E' => 4,
            'F' => 5,
            'G' => 7,
            'A' => 9,
            'B' => 11,
            _ => 0,
        }
    }

    /// Calculate frequency in Hz for a note
    fn calculate_frequency(&self, semitone: i8, modifier: i8) -> f32 {
        // MIDI note number: C4 (middle C) = 60
        // Octave 3 starts with middle C in ANSI music, so octave 4 = MIDI octave 4
        // MIDI note = (octave + 1) * 12 + semitone + modifier
        let midi_note = ((self.octave as i16 + 1) * 12 + semitone as i16 + modifier as i16) as f32;

        // Frequency = 440 * 2^((midi_note - 69) / 12)
        // A4 (MIDI 69) = 440 Hz
        440.0 * 2.0_f32.powf((midi_note - 69.0) / 12.0)
    }

    /// Convert absolute note number (1-84) to frequency
    fn note_number_to_frequency(&self, note_num: u8) -> f32 {
        // Note 1 = C in octave 0, Note 84 = B in octave 6
        // This maps to MIDI notes 12-95
        let midi_note = note_num as f32 + 11.0;
        440.0 * 2.0_f32.powf((midi_note - 69.0) / 12.0)
    }

    /// Calculate duration in milliseconds
    fn calculate_duration(&self, length: u8, dots: u32) -> u32 {
        // Duration of a whole note at current tempo
        // At 120 BPM (quarter notes per minute), a quarter note = 500ms
        // So a whole note = 2000ms at 120 BPM
        let whole_note_ms = (4.0 * 60000.0 / self.tempo as f32) as u32;

        // Base duration for the note length
        let mut duration = whole_note_ms / length as u32;

        // Apply dots (each dot adds half of the previous value)
        let mut dot_value = duration / 2;
        for _ in 0..dots {
            duration += dot_value;
            dot_value /= 2;
        }

        // Apply style
        match self.style {
            0 => duration * 7 / 8,  // Normal - 7/8 of duration
            1 => duration,          // Legato - full duration
            2 => duration * 3 / 4,  // Staccato - 3/4 of duration
            _ => duration,
        }
    }
}

/// Extract ANSI music sequences from incoming data
/// Returns (clean_data, music_notes) where clean_data has music sequences removed
pub fn extract_music(data: &str) -> (String, Vec<Vec<MusicNote>>) {
    let mut result = String::new();
    let mut all_notes = Vec::new();
    let mut parser = AnsiMusicParser::new();
    let mut remaining = data;

    while let Some((start, content_end, consumed_end)) =
        AnsiMusicParser::find_sequence(remaining)
    {
        // Add text before the sequence
        result.push_str(&remaining[..start]);

        // The payload is everything after `ESC [`, up to the boundary find_sequence
        // reported. Both indices are char boundaries by construction.
        let sequence = &remaining[start + 2..content_end];
        let notes = parser.parse(sequence);
        if !notes.is_empty() {
            all_notes.push(notes);
        }

        remaining = &remaining[consumed_end..];
    }

    // Add any remaining text
    result.push_str(remaining);

    (result, all_notes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_notes() {
        let mut parser = AnsiMusicParser::new();
        let notes = parser.parse("CDEFGAB");
        assert_eq!(notes.len(), 7);
        // C4 should be approximately 261.63 Hz
        assert!((notes[0].frequency - 261.63).abs() < 1.0);
    }

    #[test]
    fn test_parse_octave_change() {
        let mut parser = AnsiMusicParser::new();
        let notes = parser.parse("O3CO5C");
        assert_eq!(notes.len(), 2);
        // O3 C should be lower than O5 C
        assert!(notes[0].frequency < notes[1].frequency);
    }

    #[test]
    fn test_parse_sharp_flat() {
        let mut parser = AnsiMusicParser::new();
        let notes = parser.parse("CC#C-");
        assert_eq!(notes.len(), 3);
        // C < C# > C-
        assert!(notes[0].frequency < notes[1].frequency);
        assert!(notes[2].frequency < notes[0].frequency);
    }

    #[test]
    fn test_parse_rest() {
        let mut parser = AnsiMusicParser::new();
        let notes = parser.parse("CP4C");
        assert_eq!(notes.len(), 3);
        assert_eq!(notes[1].frequency, 0.0); // Rest has 0 frequency
    }

    #[test]
    fn test_find_sequence() {
        let data = "Hello\x1b[MCDEFGAB\x0eWorld";
        let result = AnsiMusicParser::find_sequence(data);
        assert!(result.is_some());
        let (start, _content_end, end) = result.unwrap();
        assert_eq!(start, 5);
        assert_eq!(&data[end..], "World");
    }

    #[test]
    fn test_extract_music() {
        let data = "Before\x1b[MCDE\x0eAfter";
        let (clean, notes) = extract_music(data);
        assert_eq!(clean, "BeforeAfter");
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].len(), 3);
    }

    /// `ESC[M` is also a perfectly ordinary CSI (Delete Lines). The no-terminator
    /// fallback used to accept the whole rest of the chunk as "music" if *any* character
    /// in it was one of T/L/O/A-G/a-g — true of essentially all English text — so a
    /// single `ESC[M` silently deleted everything after it from the display.
    #[test]
    fn plain_text_after_esc_bracket_m_is_not_swallowed_as_music() {
        let data = "\x1b[M and then some ordinary text here";
        let (text, _notes) = extract_music(data);
        assert!(text.contains("ordinary text here"),
            "text after ESC[M must not be consumed as music, got {text:?}");
    }

    /// The consumed range must land on char boundaries. With a newline terminator the
    /// caller sliced `..end - 1`, which lands inside a multi-byte character whenever one
    /// precedes the newline — a panic on ordinary UTF-8 MUD output.
    #[test]
    fn multibyte_char_before_terminator_does_not_panic() {
        let (_text, _notes) = extract_music("\x1b[MT100\u{e9}\n");
        let (_t2, _n2) = extract_music("\x1b[MT100\u{e9}");
    }

    /// `..end - 1` also dropped the final character of the sequence for the two
    /// terminators that are not themselves part of the consumed range.
    #[test]
    fn newline_terminated_sequence_keeps_its_last_note() {
        let with_nl = extract_music("\x1b[MT100CDE\n").1;
        let with_ctrl_n = extract_music("\x1b[MT100CDE\x0e").1;
        assert_eq!(with_nl.first().map(|n| n.len()), with_ctrl_n.first().map(|n| n.len()),
            "a newline-terminated sequence must yield the same notes as a Ctrl-N one");
    }

    /// Guard the other direction: tightening the fallback must not stop genuine music
    /// from being recognised, including a sequence cut off by the end of a chunk.
    #[test]
    fn genuine_music_still_parses_including_unterminated() {
        // Ctrl-N terminated
        let (text, notes) = extract_music("before\x1b[MT120L4CDEFG\x0eafter");
        assert_eq!(text, "beforeafter", "the sequence must be removed from the text");
        assert!(!notes.is_empty(), "Ctrl-N terminated music must parse");

        // newline terminated - newline stays in the stream
        let (text, notes) = extract_music("\x1b[MT120CDE\nrest");
        assert!(text.starts_with('\n'), "the newline terminator stays: {text:?}");
        assert!(!notes.is_empty(), "newline terminated music must parse");

        // unterminated but unambiguously music: still accepted
        let (text, notes) = extract_music("\x1b[MT120L4CDEFGAB");
        assert_eq!(text, "", "an all-MML remainder is still taken as music");
        assert!(!notes.is_empty());
    }
}
