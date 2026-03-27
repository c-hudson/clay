// Rendering/UI functions extracted from main.rs
// Handles terminal output rendering, ratatui frame composition, and popup rendering.

use crossterm::cursor;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::{
    App, World, OutputLine, CachedNow, Settings,
    EditorSide, EditorFocus,
    WsMessage,
    strip_ansi_codes, display_width, chars_for_display_width,
    is_ansi_only_line, is_visually_empty, has_background_color,
    colorize_square_emojis, wrap_urls_with_osc8, convert_discord_emojis_with_links,
    strip_mud_tag, convert_temperatures, get_current_time_12hr, color_name_to_ansi_bg,
    compile_action_patterns, line_matches_compiled_patterns,
    is_debug_enabled, output_debug_log,
    popup,
};

// Break characters for word wrapping within long words
const BREAK_CHARS: &[char] = &[']', ')', ',', '\\', '/', '-', '_', '&', '=', '?', ';'];

// Wrap a line with ANSI codes by visible width, preferring to break at word boundaries
// Similar to CSS white-space: pre-wrap; word-wrap: break-word
// Optimized to track byte positions instead of cloning the full string at every break point
// (still clones the small active_codes vector, but that's typically 0-5 elements)
pub(crate) fn wrap_ansi_line(line: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 {
        return vec![line.to_string()];
    }

    let mut result = Vec::new();
    let mut current_line = String::new();
    let mut current_width = 0;
    let mut in_csi = false;      // CSI sequence: \x1b[...
    let mut in_osc = false;      // OSC sequence: \x1b]...
    let mut escape_seq = String::new();
    let mut active_codes: Vec<String> = Vec::new();
    let mut active_hyperlink: Option<String> = None;  // Track active OSC 8 hyperlink URL

    // Track last word boundary (space) for wrapping - byte position instead of string clone
    let mut last_space_byte_pos: usize = 0;
    let mut last_space_width = 0;
    let mut last_space_codes: Vec<String> = Vec::new();  // small vec, ok to clone
    let mut last_space_hyperlink: Option<String> = None;
    let mut has_space_on_line = false;

    // Track break opportunities within long words (for BREAK_CHARS)
    let mut last_break_byte_pos: usize = 0;
    let mut last_break_width = 0;
    let mut last_break_codes: Vec<String> = Vec::new();
    let mut last_break_hyperlink: Option<String> = None;

    // Helper to build line prefix (color codes + hyperlink)
    // Using BEL (0x07) as OSC terminator for better terminal compatibility
    let build_prefix = |codes: &[String], hyperlink: &Option<String>| -> String {
        let mut prefix = codes.join("");
        if let Some(url) = hyperlink {
            prefix.push_str("\x1b]8;;");
            prefix.push_str(url);
            prefix.push('\x07');
        }
        prefix
    };

    // Helper to build line suffix (close hyperlink + reset colors)
    let build_suffix = |hyperlink: &Option<String>| -> String {
        let mut suffix = String::new();
        if hyperlink.is_some() {
            suffix.push_str("\x1b]8;;\x07");
        }
        suffix.push_str("\x1b[0m");
        suffix
    };

    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];

        // Handle CSI/OSC continuations first, before checking for new ESC
        if in_csi {
            escape_seq.push(c);
            // CSI ends with alphabetic char or ~
            if c.is_alphabetic() || c == '~' {
                in_csi = false;
                current_line.push_str(&escape_seq);
                if c == 'm' {
                    if escape_seq == "\x1b[0m" || escape_seq == "\x1b[m" {
                        active_codes.clear();
                    } else {
                        active_codes.push(escape_seq.clone());
                    }
                }
                escape_seq.clear();
            }
            i += 1;
            continue;
        } else if in_osc {
            // OSC ends with BEL (\x07) or ST (\x1b\\)
            if c == '\x07' {
                escape_seq.push(c);
                in_osc = false;
                // Check if this is an OSC 8 hyperlink
                if escape_seq.starts_with("\x1b]8;;") {
                    let url = &escape_seq[5..escape_seq.len()-1];
                    if url.is_empty() {
                        active_hyperlink = None;
                    } else {
                        active_hyperlink = Some(url.to_string());
                    }
                }
                current_line.push_str(&escape_seq);
                escape_seq.clear();
                i += 1;
                continue;
            } else if c == '\x1b' && i + 1 < chars.len() && chars[i + 1] == '\\' {
                // ST terminator
                escape_seq.push(c);
                escape_seq.push('\\');
                in_osc = false;
                // Check if this is an OSC 8 hyperlink
                if escape_seq.starts_with("\x1b]8;;") {
                    let url = &escape_seq[5..escape_seq.len()-2];
                    if url.is_empty() {
                        active_hyperlink = None;
                    } else {
                        active_hyperlink = Some(url.to_string());
                    }
                }
                current_line.push_str(&escape_seq);
                escape_seq.clear();
                i += 2;
                continue;
            } else {
                escape_seq.push(c);
                i += 1;
                continue;
            }
        } else if c == '\x1b' {
            escape_seq.push(c);
            // Check next char to determine sequence type
            if i + 1 < chars.len() {
                let next = chars[i + 1];
                if next == '[' {
                    in_csi = true;
                    escape_seq.push(next);
                    i += 2;
                    continue;
                } else if next == ']' {
                    in_osc = true;
                    escape_seq.push(next);
                    i += 2;
                    continue;
                } else if next == '\\' {
                    // ST (String Terminator) - standalone, pass through
                    escape_seq.push(next);
                    current_line.push_str(&escape_seq);
                    escape_seq.clear();
                    i += 2;
                    continue;
                }
            }
            // Lone ESC or unknown sequence
            current_line.push(c);
            escape_seq.clear();
            i += 1;
            continue;
        } else {
            // Tab characters are rendered as 8 spaces (see process_output_line),
            // but unicode_width returns None/0 for tabs. Use 8 to match rendering.
            let char_width = if c == '\t' { 8 } else {
                unicode_width::UnicodeWidthChar::width(c).unwrap_or(0)
            };

            // Check if we need to wrap before adding this character
            if current_width + char_width > max_width && current_width > 0 {
                // Prefer break char over space when it's further along (packs more on line)
                if last_break_width > last_space_width {
                    // Break at period/hyphen/etc within the current word
                    let mut break_line = current_line[..last_break_byte_pos].to_string();
                    break_line.push_str(&build_suffix(&last_break_hyperlink));
                    result.push(break_line);

                    let prefix = build_prefix(&last_break_codes, &last_break_hyperlink);
                    let remainder = &current_line[last_break_byte_pos..];
                    current_line = prefix + remainder;
                    current_width -= last_break_width;
                } else if has_space_on_line && last_space_width > 0 {
                    // Break at word boundary - emit up to and including the space
                    let mut break_line = current_line[..last_space_byte_pos].to_string();
                    break_line.push_str(&build_suffix(&last_space_hyperlink));
                    result.push(break_line);

                    // Start new line with content after the space
                    let prefix = build_prefix(&last_space_codes, &last_space_hyperlink);
                    let remainder = &current_line[last_space_byte_pos..];
                    current_line = prefix + remainder;
                    current_width -= last_space_width;
                } else {
                    // No break point at all - hard wrap at current position
                    current_line.push_str(&build_suffix(&active_hyperlink));
                    result.push(current_line);
                    current_line = build_prefix(&active_codes, &active_hyperlink);
                    current_width = 0;
                }

                // Reset tracking for new line
                has_space_on_line = false;
                last_space_byte_pos = 0;
                last_space_width = 0;
                last_space_codes.clear();
                last_space_hyperlink = None;
                last_break_byte_pos = 0;
                last_break_width = 0;
                last_break_codes.clear();
                last_break_hyperlink = None;
            }

            // Add the character
            current_line.push(c);
            current_width += char_width;

            // Track break opportunities - save byte position (not string clone)
            if c.is_whitespace() {
                // Save position AFTER the space as a word boundary
                last_space_byte_pos = current_line.len();
                last_space_width = current_width;
                last_space_codes = active_codes.clone();  // small vec
                last_space_hyperlink = active_hyperlink.clone();
                has_space_on_line = true;
                // Reset break char tracking since we have a new word
                last_break_byte_pos = 0;
                last_break_width = 0;
            } else if BREAK_CHARS.contains(&c) {
                // Save as potential break point within a word (after this char)
                last_break_byte_pos = current_line.len();
                last_break_width = current_width;
                last_break_codes = active_codes.clone();
                last_break_hyperlink = active_hyperlink.clone();
            }

            i += 1;
        }
    }

    if !current_line.is_empty() || result.is_empty() {
        // Add suffix to close any active hyperlink and reset colors
        current_line.push_str(&build_suffix(&active_hyperlink));
        result.push(current_line);
    }

    result
}

pub(crate) fn ui(f: &mut Frame, app: &mut App) {
    let total_height = f.size().height.max(3);  // Minimum 3 lines for output + separator + input

    // Layout: output area, separator bar (1 line), input area
    let separator_height = 1;
    let input_total_height = app.input_height;
    let output_height = total_height.saturating_sub(separator_height + input_total_height);

    // Store output dimensions for scrolling and more-mode calculations
    // Use max(1) to prevent any division by zero elsewhere
    let new_output_height = output_height.max(1);
    let new_output_width = f.size().width.max(1);
    // Mark output for redraw if dimensions changed (terminal resize)
    let dimensions_changed = new_output_height != app.output_height || new_output_width != app.output_width;
    if dimensions_changed {
        app.needs_output_redraw = true;
    }
    app.output_height = new_output_height;
    app.output_width = new_output_width;
    // Send NAWS updates if terminal was resized
    if dimensions_changed {
        app.send_naws_to_all_worlds();
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(output_height),
            Constraint::Length(separator_height),
            Constraint::Length(input_total_height),
        ])
        .split(f.size());

    let output_area = chunks[0];
    let separator_area = chunks[1];
    let input_area = chunks[2];

    // Update input dimensions and prompt length for viewport calculation
    app.input.set_dimensions(input_area.width, app.input_height);
    app.input.prompt_len = strip_ansi_codes(&app.current_world().prompt).chars().count();

    // Check if editor is visible - split the output area if so
    if app.editor.visible {
        // Split output area horizontally into editor and world output
        let half_width = output_area.width / 2;
        let (editor_area, world_area) = match app.settings.editor_side {
            EditorSide::Left => {
                let editor = Rect {
                    x: output_area.x,
                    y: output_area.y,
                    width: half_width,
                    height: output_area.height,
                };
                let world = Rect {
                    x: output_area.x + half_width,
                    y: output_area.y,
                    width: output_area.width - half_width,
                    height: output_area.height,
                };
                (editor, world)
            }
            EditorSide::Right => {
                let world = Rect {
                    x: output_area.x,
                    y: output_area.y,
                    width: half_width,
                    height: output_area.height,
                };
                let editor = Rect {
                    x: output_area.x + half_width,
                    y: output_area.y,
                    width: output_area.width - half_width,
                    height: output_area.height,
                };
                (editor, world)
            }
        };

        // Render editor panel
        render_editor_panel(f, app, editor_area);

        // Render world output on the other half
        render_output_area(f, app, world_area);
    } else {
        // Normal full-width output area
        render_output_area(f, app, output_area);
    }

    // Render separator bar
    render_separator_bar(f, app, separator_area);

    // Render input area
    render_input_area(f, app, input_area);

    // Render popups if visible (confirm dialog last so it's on top)
    render_confirm_dialog(f, app);
    render_filter_popup(f, app);
    // New unified popup system - renders on top of old popups
    render_new_popup(f, app);
}

/// Process an output line for display. Returns None if the line should be skipped.
/// Returns Some(processed_text) with emoji colorization, client prefix, timestamp/tags, and tab expansion applied.
pub(crate) fn process_output_line(line: &OutputLine, show_tags: bool, temp_convert_enabled: bool, zwj_enabled: bool, cached_now: &CachedNow) -> Option<String> {
    // Skip gagged lines unless show_tags (F2) is enabled
    if line.gagged && !show_tags {
        return None;
    }
    // Skip lines that are only ANSI codes (cursor control garbage)
    if is_ansi_only_line(&line.text) {
        return None;
    }
    // For legitimate blank lines (empty or whitespace-only without background colors), render as blank
    if is_visually_empty(&line.text) && !has_background_color(&line.text) {
        return Some(String::new());
    }
    // Colorize square emoji (🟩🟨 etc.) with ANSI codes
    let text = colorize_square_emojis(&line.text, zwj_enabled);
    // Add ✨ prefix for client-generated messages
    let text = if !line.from_server {
        format!("✨ {}", text)
    } else {
        text
    };
    let processed = if show_tags {
        // Show timestamp + original text when tags are shown
        let text_with_temps = if temp_convert_enabled {
            convert_temperatures(&text)
        } else {
            text
        };
        format!("\x1b[36m{}\x1b[0m {}", line.format_timestamp_with_now(cached_now), text_with_temps)
    } else {
        strip_mud_tag(&text)
    };
    Some(processed.replace('\t', "        "))
}

/// A single visual line as it would appear in the output display.
/// Used by `build_display_lines()` for testable display logic.
#[derive(Debug, Clone)]
pub struct DisplayLine {
    /// The text content of the visual line (may contain ANSI sequences)
    pub text: String,
    /// True if this line is marked as new/unseen
    pub marked_new: bool,
    /// True if this line matches highlight actions (F8)
    pub highlight_f8: bool,
    /// Optional highlight color from /highlight action
    pub highlight_color: Option<String>,
}

/// Build the list of visual lines that would be displayed in the output area.
/// This is the pure, testable core of `render_output_crossterm` — it computes
/// which lines are visible given the world state, settings, and terminal dimensions,
/// but does NOT perform any terminal I/O.
pub fn build_display_lines(
    world: &World,
    settings: &Settings,
    visible_height: usize,
    term_width: usize,
    show_tags: bool,
) -> Vec<DisplayLine> {
    let temp_convert_enabled = settings.temp_convert_enabled;
    let zwj_enabled = settings.zwj_enabled;
    let new_line_indicator = settings.new_line_indicator;
    let nli_prefix_width: usize = 2;
    let min_old_context: usize = if new_line_indicator { 2 } else { 0 };
    let cached_now = CachedNow::new();

    let expand_and_wrap = |line: &OutputLine, term_width: usize, show_tags: bool, highlight_f8: bool, cached_now: &CachedNow| -> Vec<(String, bool, Option<String>, bool)> {
        let expanded = match process_output_line(line, show_tags, temp_convert_enabled, zwj_enabled, cached_now) {
            Some(text) if text.is_empty() => return vec![("".to_string(), false, None, false)],
            Some(text) => text,
            None => return Vec::new(),
        };
        let with_links = wrap_urls_with_osc8(&expanded);
        let with_emoji_links = if show_tags {
            with_links
        } else {
            convert_discord_emojis_with_links(&with_links)
        };
        let hl_color = line.highlight_color.clone();
        let mn = line.marked_new;
        let wrap_width = if new_line_indicator && mn {
            term_width.saturating_sub(nli_prefix_width)
        } else {
            term_width
        };
        wrap_ansi_line(&with_emoji_links, wrap_width)
            .into_iter()
            .map(|s| (s, highlight_f8, hl_color.clone(), mn))
            .collect()
    };

    if world.output_lines.is_empty() {
        return Vec::new();
    }

    // Normal unfiltered rendering
    let end_line = world.scroll_offset.min(world.output_lines.len().saturating_sub(1));

    let mut rev_lines: Vec<(String, bool, Option<String>, bool)> = Vec::with_capacity(visible_height + 8);
    let mut first_line_idx = end_line;
    let mut old_visual_count: usize = 0;
    let mut applied_vlo = false;
    for line_idx in (0..=end_line).rev() {
        first_line_idx = line_idx;
        let line = &world.output_lines[line_idx];
        // No highlight action matching in this pure function (would need compiled patterns)
        let highlight = false;
        let mut wrapped = expand_and_wrap(line, term_width, show_tags, highlight, &cached_now);

        if !applied_vlo && world.visual_line_offset > 0
           && !wrapped.is_empty() && wrapped.len() > world.visual_line_offset {
            wrapped.truncate(world.visual_line_offset);
            applied_vlo = true;
        }

        let is_new = line.marked_new;
        for w in wrapped.into_iter().rev() {
            rev_lines.push(w);
        }
        if !is_new {
            old_visual_count += 1;
        }

        if rev_lines.len() >= visible_height
            && (!new_line_indicator || old_visual_count >= min_old_context)
        {
            break;
        }
        if rev_lines.len() >= visible_height * 2 {
            break;
        }
    }
    rev_lines.reverse();
    let mut visual_lines = rev_lines;

    if visual_lines.len() < visible_height && first_line_idx == 0 {
        for line_idx in (end_line + 1)..world.output_lines.len() {
            let line = &world.output_lines[line_idx];
            let highlight = false;
            let wrapped = expand_and_wrap(line, term_width, show_tags, highlight, &cached_now);

            for w in wrapped {
                visual_lines.push(w);
            }

            if visual_lines.len() >= visible_height {
                break;
            }
        }
    }

    // Build the final display slice (NLI composition logic)
    let final_lines: Vec<(String, bool, Option<String>, bool)> = if visual_lines.len() <= visible_height {
        visual_lines
    } else if new_line_indicator && visual_lines.len() <= visible_height + min_old_context {
        let mut old_prefix = 0;
        for (_,_,_,mn) in &visual_lines {
            if !mn { old_prefix += 1; } else { break; }
        }
        let context = old_prefix.min(min_old_context);
        if context > 0 {
            let newest_count = visible_height - context;
            let mut composed = Vec::new();
            composed.extend_from_slice(&visual_lines[..context]);
            let tail_start = visual_lines.len().saturating_sub(newest_count);
            composed.extend_from_slice(&visual_lines[tail_start..]);
            composed
        } else {
            let start = visual_lines.len().saturating_sub(visible_height);
            visual_lines[start..].to_vec()
        }
    } else {
        let start = visual_lines.len().saturating_sub(visible_height);
        visual_lines[start..].to_vec()
    };

    final_lines.into_iter().map(|(text, highlight_f8, highlight_color, marked_new)| {
        DisplayLine { text, marked_new, highlight_f8, highlight_color }
    }).collect()
}

/// Render output area using raw crossterm (bypasses ratatui's buggy rendering)
/// Returns early if splash screen, popup, or editor is visible (let ratatui handle those)
pub(crate) fn render_output_crossterm(app: &App) {
    use std::io::Write;
    use crossterm::{style::Print, QueueableCommand};

    // Skip if showing splash screen or editor is visible
    // When editor is visible, ratatui handles all rendering for the split-screen layout
    if app.current_world().showing_splash || app.editor.visible {
        return;
    }

    // Skip if any overlay popup is visible - ratatui handles output rendering behind popups
    if app.has_new_popup() || app.confirm_dialog.visible {
        return;
    }

    let mut stdout = std::io::stdout();

    let world = app.current_world();
    let visible_height = (app.output_height as usize).max(1);
    let term_width = (app.output_width as usize).max(1);

    // Calculate visible width of a string (excluding ANSI escape sequences including OSC 8)
    fn visible_width(s: &str) -> usize {
        let mut width = 0;
        let mut in_csi = false;   // CSI sequence: \x1b[...
        let mut in_osc = false;   // OSC sequence: \x1b]...
        let chars: Vec<char> = s.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            // Handle CSI/OSC continuations first, before checking for new ESC
            if in_csi {
                if c.is_alphabetic() || c == '~' {
                    in_csi = false;
                }
                i += 1;
                continue;
            } else if in_osc {
                // OSC ends with BEL or ST (\x1b\\)
                if c == '\x07' {
                    in_osc = false;
                    i += 1;
                    continue;
                } else if c == '\x1b' && i + 1 < chars.len() && chars[i + 1] == '\\' {
                    in_osc = false;
                    i += 2;
                    continue;
                }
                i += 1;
                continue;
            } else if c == '\x1b' {
                // Start of new escape sequence
                if i + 1 < chars.len() {
                    let next = chars[i + 1];
                    if next == '[' {
                        in_csi = true;
                        i += 2;
                        continue;
                    } else if next == ']' {
                        in_osc = true;
                        i += 2;
                        continue;
                    } else if next == '\\' {
                        // ST terminator (standalone)
                        i += 2;
                        continue;
                    }
                }
                i += 1;
                continue;
            } else {
                width += unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
                i += 1;
            }
        }
        width
    }

    // Collect wrapped lines centered around scroll_offset to fill the screen
    // Each entry is (line_text, should_highlight_f8, highlight_color_from_action, marked_new)
    let mut visual_lines: Vec<(String, bool, Option<String>, bool)> = Vec::new();
    let mut first_line_idx: usize = 0;

    let show_tags = app.show_tags;
    let temp_convert_enabled = app.settings.temp_convert_enabled;
    let zwj_enabled = app.settings.zwj_enabled;
    let highlight_actions = app.highlight_actions;
    let world_name = &world.name;
    // Pre-compile action patterns once (not per-line)
    let compiled_patterns = if highlight_actions {
        compile_action_patterns(world_name, &app.settings.actions)
    } else {
        Vec::new()
    };

    // Cache "now" for timestamp formatting - compute once per frame, not per line
    let cached_now = CachedNow::new();

    let new_line_indicator = app.settings.new_line_indicator;
    // The "▶ " prefix is 2 columns wide (1 for triangle + 1 for space)
    let nli_prefix_width: usize = 2;
    // Minimum old (non-new) context lines to show at top when switching worlds
    let min_old_context: usize = if new_line_indicator { 2 } else { 0 };
    let expand_and_wrap = |line: &OutputLine, term_width: usize, show_tags: bool, highlight_f8: bool, cached_now: &CachedNow| -> Vec<(String, bool, Option<String>, bool)> {
        let expanded = match process_output_line(line, show_tags, temp_convert_enabled, zwj_enabled, cached_now) {
            Some(text) if text.is_empty() => return vec![("".to_string(), false, None, false)],
            Some(text) => text,
            None => return Vec::new(),
        };
        // Wrap URLs with OSC 8 hyperlink sequences for terminal clickability
        let with_links = wrap_urls_with_osc8(&expanded);
        // Convert Discord custom emojis to clickable :name: links (after URL wrapping to avoid conflicts)
        // Skip conversion when show_tags is enabled so users can see original text
        let with_emoji_links = if show_tags {
            with_links
        } else {
            convert_discord_emojis_with_links(&with_links)
        };
        let hl_color = line.highlight_color.clone();
        let mn = line.marked_new;
        // Reduce wrap width when new line indicator prefix will be prepended
        let wrap_width = if new_line_indicator && mn {
            term_width.saturating_sub(nli_prefix_width)
        } else {
            term_width
        };
        wrap_ansi_line(&with_emoji_links, wrap_width)
            .into_iter()
            .map(|s| (s, highlight_f8, hl_color.clone(), mn))
            .collect()
    };

    // Helper to check if a line should be highlighted
    let should_highlight = |line: &OutputLine| -> bool {
        highlight_actions && line_matches_compiled_patterns(&line.text, &compiled_patterns)
    };

    // Check if filter popup is active with a non-empty filter
    // If filter is active but no matches, show nothing (don't fall through to normal rendering)
    if app.filter_popup.visible && !app.filter_popup.filter_text.is_empty() {
        // If no matches, visual_lines stays empty - that's correct behavior
        if !app.filter_popup.filtered_indices.is_empty() {
            // Use filtered indices
            let filtered = &app.filter_popup.filtered_indices;
            let end_pos = app.filter_popup.scroll_offset.min(filtered.len().saturating_sub(1));

            // Work backwards from scroll_offset to fill the screen
            // Collect in reverse, then reverse once (avoids O(n²) insert(0, ...))
            let mut rev_lines: Vec<(String, bool, Option<String>, bool)> = Vec::with_capacity(visible_height + 8);
            for pos in (0..=end_pos).rev() {
                let line_idx = filtered[pos];
                if line_idx < world.output_lines.len() {
                    let line = &world.output_lines[line_idx];
                    let highlight = should_highlight(line);
                    let wrapped = expand_and_wrap(line, term_width, show_tags, highlight, &cached_now);

                    for w in wrapped.into_iter().rev() {
                        rev_lines.push(w);
                    }

                    if rev_lines.len() >= visible_height {
                        break;
                    }
                }
            }
            rev_lines.reverse();
            visual_lines = rev_lines;

            // If we still have room, show lines after scroll_offset
            if visual_lines.len() < visible_height {
                for &line_idx in filtered.iter().skip(end_pos + 1) {
                    if line_idx < world.output_lines.len() {
                        let line = &world.output_lines[line_idx];
                        let highlight = should_highlight(line);
                        let wrapped = expand_and_wrap(line, term_width, show_tags, highlight, &cached_now);

                        for w in wrapped {
                            visual_lines.push(w);
                        }

                        if visual_lines.len() >= visible_height {
                            break;
                        }
                    }
                }
            }
        }
    } else if !world.output_lines.is_empty() {
        // Normal unfiltered rendering
        let end_line = world.scroll_offset.min(world.output_lines.len().saturating_sub(1));

        // Collect lines in reverse order, then reverse once (avoids O(n²) insert(0, ...))
        let mut rev_lines: Vec<(String, bool, Option<String>, bool)> = Vec::with_capacity(visible_height + 8);
        first_line_idx = end_line;
        let mut old_visual_count: usize = 0;
        let mut applied_vlo = false;
        for line_idx in (0..=end_line).rev() {
            first_line_idx = line_idx;
            let line = &world.output_lines[line_idx];
            let highlight = should_highlight(line);
            let mut wrapped = expand_and_wrap(line, term_width, show_tags, highlight, &cached_now);

            // Partial line display: truncate the first visible line encountered from the end.
            // This may not be end_line itself if gagged lines were appended after the trigger.
            if !applied_vlo && world.visual_line_offset > 0
               && !wrapped.is_empty() && wrapped.len() > world.visual_line_offset {
                wrapped.truncate(world.visual_line_offset);
                applied_vlo = true;
            }

            let is_new = line.marked_new;
            for w in wrapped.into_iter().rev() {
                rev_lines.push(w);
            }
            if !is_new {
                old_visual_count += 1;
            }

            // Collect enough to fill screen. When NLI is enabled, keep going
            // past visible_height until we have min_old_context non-new lines,
            // so the display window can include them at the top.
            if rev_lines.len() >= visible_height
                && (!new_line_indicator || old_visual_count >= min_old_context)
            {
                break;
            }
            // Safety limit: don't collect more than 2x screen height
            if rev_lines.len() >= visible_height * 2 {
                break;
            }
        }
        rev_lines.reverse();
        visual_lines = rev_lines;

        if visual_lines.len() < visible_height && first_line_idx == 0 {
            for line_idx in (end_line + 1)..world.output_lines.len() {
                let line = &world.output_lines[line_idx];
                let highlight = should_highlight(line);
                let wrapped = expand_and_wrap(line, term_width, show_tags, highlight, &cached_now);

                for w in wrapped {
                    visual_lines.push(w);
                }

                if visual_lines.len() >= visible_height {
                    break;
                }
            }
        }
    }

    // Debug: verify output line sequence order (only check visible range, log mismatches)
    if is_debug_enabled() && !world.output_lines.is_empty() {
        let check_start = first_line_idx;
        let check_end = world.scroll_offset.min(world.output_lines.len().saturating_sub(1));
        let mut last_seq: Option<u64> = None;
        for idx in check_start..=check_end {
            let line = &world.output_lines[idx];
            if line.gagged && !show_tags {
                continue; // Skip gagged lines when not showing tags
            }
            if let Some(prev_seq) = last_seq {
                if line.seq <= prev_seq {
                    if let Some(ref tx) = app.ws_client_tx {
                        // Remote console: report to master via WebSocket
                        let _ = tx.send(WsMessage::ReportSeqMismatch {
                            world_index: app.current_world_index,
                            expected_seq_gt: prev_seq,
                            actual_seq: line.seq,
                            line_text: line.text.chars().take(80).collect::<String>(),
                            source: "console".to_string(),
                        });
                    } else {
                        // Local console: write to debug file
                        output_debug_log(&format!("SEQ MISMATCH in '{}': idx={}, expected seq>{}, got seq={}, text={:?}",
                            world.name, idx, prev_seq, line.seq,
                            line.text.chars().take(80).collect::<String>()));
                    }
                }
            }
            last_seq = Some(line.seq);
        }
    }

    // Build the final display slice. When NLI is enabled and the world is paused
    // (has pending lines), compose: old context at top + newest lines at bottom,
    // dropping middle lines so total fits in visible_height. Once pending is fully
    // released, just show the bottom visible_height lines normally.
    let mut composed_lines: Vec<(String, bool, Option<String>, bool)> = Vec::new();
    let lines_to_show: &[(String, bool, Option<String>, bool)] = if visual_lines.len() <= visible_height {
        &visual_lines[..visual_lines.len()]
    } else if new_line_indicator && visual_lines.len() <= visible_height + min_old_context {
        // Old context lines are close to the bottom (within one screen + context).
        // Compose: old context at top + newest lines at bottom.
        let mut old_prefix = 0;
        for (_,_,_,mn) in &visual_lines {
            if !mn { old_prefix += 1; } else { break; }
        }
        let context = old_prefix.min(min_old_context);
        if context > 0 {
            let newest_count = visible_height - context;
            composed_lines.extend_from_slice(&visual_lines[..context]);
            let tail_start = visual_lines.len().saturating_sub(newest_count);
            composed_lines.extend_from_slice(&visual_lines[tail_start..]);
            &composed_lines
        } else {
            let start = visual_lines.len().saturating_sub(visible_height);
            &visual_lines[start..]
        }
    } else {
        let start = visual_lines.len().saturating_sub(visible_height);
        &visual_lines[start..]
    };

    for (row_idx, (wrapped, highlight_f8, hl_color, marked_new)) in lines_to_show.iter().enumerate() {
        let row_y = row_idx as u16;

        let _ = stdout.queue(cursor::MoveTo(0, row_y));

        // New line indicator: green triangle prefix for unseen/pending lines
        if new_line_indicator && *marked_new {
            let _ = stdout.queue(Print("\x1b[32m▶\x1b[0m "));
        }

        // Determine background color: /highlight color takes priority, then F8 highlight
        let bg_code = if let Some(color) = hl_color {
            Some(color_name_to_ansi_bg(color))
        } else if *highlight_f8 {
            // Dark yellow/brown background for F8 action-matched lines
            Some("\x1b[48;5;58m".to_string())
        } else {
            None
        };

        if let Some(ref bg) = bg_code {
            let _ = stdout.queue(Print(bg));
            // Replace \x1b[0m with \x1b[0m<bg_code> to preserve background
            let highlighted = wrapped.replace("\x1b[0m", &format!("\x1b[0m{}", bg));
            let _ = stdout.queue(Print(&highlighted));
            // Pad with spaces so the highlight background extends to end of line
            let line_visible_width = visible_width(wrapped);
            if line_visible_width < term_width {
                let padding = " ".repeat(term_width - line_visible_width);
                let _ = stdout.queue(Print(padding));
            }
        } else {
            let _ = stdout.queue(Print(wrapped));
        }

        // Reset colors and erase to end of line - more robust than space padding
        // for preventing background color bleed on macOS terminals
        let _ = stdout.queue(Print("\x1b[0m\x1b[K"));
    }

    for row_idx in lines_to_show.len()..visible_height {
        let row_y = row_idx as u16;
        let _ = stdout.queue(cursor::MoveTo(0, row_y));
        let _ = stdout.queue(Print("\x1b[K"));
    }

    // Render filter popup if visible (must be after output so it's on top)
    if app.filter_popup.visible {
        let popup_width = 40usize.min(term_width);
        let x = term_width.saturating_sub(popup_width) as u16;
        let title = " Find [Esc to close] ";
        let dashes_needed = popup_width.saturating_sub(title.len() + 2); // +2 for corners

        // Draw border top
        let _ = stdout.queue(cursor::MoveTo(x, 0));
        let border_top = format!("\x1b[36m┌{}{}{}\x1b[0m", title, "─".repeat(dashes_needed), "┐");
        let _ = stdout.queue(Print(border_top));

        // Draw content line
        let _ = stdout.queue(cursor::MoveTo(x, 1));
        let mut display_text = app.filter_popup.filter_text.clone();
        display_text.insert(app.filter_popup.cursor, '▏');
        let label = "Filter: ";
        let content_width = label.len() + display_text.chars().count();
        let inner_width = popup_width.saturating_sub(2); // -2 for side borders
        let padding = inner_width.saturating_sub(content_width);
        let _ = stdout.queue(Print(format!("\x1b[36m│\x1b[0m{}{}{}\x1b[36m│\x1b[0m", label, display_text, " ".repeat(padding))));

        // Draw border bottom
        let _ = stdout.queue(cursor::MoveTo(x, 2));
        let border_bottom = format!("\x1b[36m└{}┘\x1b[0m", "─".repeat(popup_width.saturating_sub(2)));
        let _ = stdout.queue(Print(border_bottom));
    }

    // Calculate and set cursor position in input area
    // This replicates the logic from render_input_area to avoid Save/Restore timing issues
    let prompt = &app.current_world().prompt;
    let prompt_len = strip_ansi_codes(prompt).chars().count();
    let cursor_line = app.input.cursor_line();
    let viewport_line = cursor_line.saturating_sub(app.input.viewport_start_line);

    // Input area starts after output + separator bar (1 line)
    let input_area_y = app.output_height + 1;
    let input_area_width = term_width.max(1);

    if viewport_line < app.input_height as usize {
        // Calculate cursor column accounting for newlines in the buffer
        let first_line_capacity = input_area_width.saturating_sub(prompt_len);
        let text_before_cursor = &app.input.buffer[..app.input.cursor_position];

        let mut col_width = 0usize;
        let mut is_first_line = true;

        for c in text_before_cursor.chars() {
            if c == '\n' {
                col_width = 0;
                is_first_line = false;
                continue;
            }
            let cw = display_width(&c.to_string());
            let capacity = if is_first_line { first_line_capacity } else { input_area_width };
            col_width += cw;
            if capacity > 0 && col_width >= capacity {
                if col_width == capacity {
                    col_width = 0;
                } else {
                    col_width = cw;
                }
                is_first_line = false;
            }
        }

        let cursor_col = if is_first_line && app.input.viewport_start_line == 0 {
            col_width + prompt_len
        } else {
            col_width
        };

        let cursor_x = cursor_col as u16;
        // Calculate visual line within viewport
        let cursor_y = input_area_y + viewport_line as u16;
        let max_y = input_area_y + app.input_height - 1;
        let _ = stdout.queue(cursor::MoveTo(cursor_x, cursor_y.min(max_y)));
    }

    let _ = stdout.flush();
}

/// Write an ANSI-colored string directly to a ratatui buffer at (x, y).
/// Parses ANSI SGR escape sequences and sets cell styles accordingly.
/// This bypasses ansi_to_tui and Paragraph for reliable color reproduction.
pub(crate) fn ansi_string_to_buffer(buf: &mut ratatui::buffer::Buffer, x: u16, y: u16, s: &str, max_width: u16) {
    use ratatui::style::Color;

    let mut col = 0u16;
    let mut style = Style::default();
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if col >= max_width {
            break;
        }

        if c == '\x1b' {
            // Parse escape sequence
            if chars.peek() == Some(&'[') {
                chars.next(); // consume '['
                let mut params = String::new();
                // Collect parameter bytes
                while let Some(&pc) = chars.peek() {
                    if pc.is_ascii_digit() || pc == ';' {
                        params.push(pc);
                        chars.next();
                    } else {
                        break;
                    }
                }
                // Get the final byte
                if let Some(final_byte) = chars.next() {
                    if final_byte == 'm' {
                        // SGR sequence - parse parameters
                        let nums: Vec<u32> = params.split(';')
                            .filter(|s| !s.is_empty())
                            .filter_map(|s| s.parse().ok())
                            .collect();
                        if nums.is_empty() {
                            // \x1b[m = reset
                            style = Style::default();
                        } else {
                            let mut i = 0;
                            while i < nums.len() {
                                match nums[i] {
                                    0 => style = Style::default(),
                                    1 => style = style.add_modifier(Modifier::BOLD),
                                    2 => style = style.add_modifier(Modifier::DIM),
                                    3 => style = style.add_modifier(Modifier::ITALIC),
                                    4 => style = style.add_modifier(Modifier::UNDERLINED),
                                    5 | 6 => style = style.add_modifier(Modifier::SLOW_BLINK),
                                    7 => style = style.add_modifier(Modifier::REVERSED),
                                    8 => style = style.add_modifier(Modifier::HIDDEN),
                                    9 => style = style.add_modifier(Modifier::CROSSED_OUT),
                                    22 => style = style.remove_modifier(Modifier::BOLD | Modifier::DIM),
                                    23 => style = style.remove_modifier(Modifier::ITALIC),
                                    24 => style = style.remove_modifier(Modifier::UNDERLINED),
                                    25 => style = style.remove_modifier(Modifier::SLOW_BLINK),
                                    27 => style = style.remove_modifier(Modifier::REVERSED),
                                    28 => style = style.remove_modifier(Modifier::HIDDEN),
                                    29 => style = style.remove_modifier(Modifier::CROSSED_OUT),
                                    // Use Indexed colors to exactly match SGR codes.
                                    // ratatui's named Color variants (Red, White, etc.) map
                                    // through crossterm to different palette indices than
                                    // the raw SGR codes, causing bright/dark mismatches.
                                    30..=37 => style = style.fg(Color::Indexed((nums[i] - 30) as u8)),
                                    38 => {
                                        // Extended foreground color
                                        if i + 1 < nums.len() && nums[i + 1] == 5 && i + 2 < nums.len() {
                                            style = style.fg(Color::Indexed(nums[i + 2] as u8));
                                            i += 2;
                                        } else if i + 1 < nums.len() && nums[i + 1] == 2 && i + 4 < nums.len() {
                                            style = style.fg(Color::Rgb(
                                                nums[i + 2] as u8,
                                                nums[i + 3] as u8,
                                                nums[i + 4] as u8,
                                            ));
                                            i += 4;
                                        }
                                    }
                                    39 => style = style.fg(Color::Reset),
                                    40..=47 => style = style.bg(Color::Indexed((nums[i] - 40) as u8)),
                                    48 => {
                                        // Extended background color
                                        if i + 1 < nums.len() && nums[i + 1] == 5 && i + 2 < nums.len() {
                                            style = style.bg(Color::Indexed(nums[i + 2] as u8));
                                            i += 2;
                                        } else if i + 1 < nums.len() && nums[i + 1] == 2 && i + 4 < nums.len() {
                                            style = style.bg(Color::Rgb(
                                                nums[i + 2] as u8,
                                                nums[i + 3] as u8,
                                                nums[i + 4] as u8,
                                            ));
                                            i += 4;
                                        }
                                    }
                                    49 => style = style.bg(Color::Reset),
                                    90..=97 => style = style.fg(Color::Indexed((nums[i] - 90 + 8) as u8)),
                                    100..=107 => style = style.bg(Color::Indexed((nums[i] - 100 + 8) as u8)),
                                    _ => {}
                                }
                                i += 1;
                            }
                        }
                    }
                    // Non-SGR CSI sequences (e.g., cursor movement) are ignored
                }
            } else if chars.peek() == Some(&']') {
                // OSC sequence - skip until BEL or ST
                chars.next(); // consume ']'
                while let Some(oc) = chars.next() {
                    if oc == '\x07' {
                        break;
                    }
                    if oc == '\x1b' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            // Other escape sequences are ignored
            continue;
        }

        // Regular character - write to buffer
        let char_width = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if char_width == 0 && c != ' ' {
            // Zero-width character (ZWJ, combining marks, etc.) - skip
            continue;
        }

        let cell_x = x + col;
        if cell_x < x + max_width {
            let cell = buf.get_mut(cell_x, y);
            cell.set_char(c);
            cell.set_style(style);
            col += char_width as u16;
            // For wide characters, mark the next cell as continuation
            if char_width == 2 && col <= max_width {
                let next_cell = buf.get_mut(x + col - 1, y);
                next_cell.set_char(' ');
                next_cell.set_style(style);
            }
        } else {
            break;
        }
    }
}

pub(crate) fn render_output_area(f: &mut Frame, app: &App, area: Rect) {
    let world = app.current_world();
    let visible_height = area.height as usize;
    let area_width = area.width as usize;

    // Check if showing splash screen - ratatui handles splash rendering
    if world.showing_splash {
        f.render_widget(ratatui::widgets::Clear, area);
        let output_text = render_splash_centered(world, visible_height, area_width);
        let output_paragraph = Paragraph::new(output_text);
        f.render_widget(output_paragraph, area);
        return;
    }

    // Check if any overlay popup is visible (setup, help, world selector, etc.)
    let has_overlay_popup = app.confirm_dialog.visible || app.has_new_popup();

    // If no overlay popup and no editor, raw crossterm will handle output rendering
    // (it provides better ANSI color handling than ratatui's ansi_to_tui conversion)
    // Clear the area so ratatui doesn't show stale content - crossterm will fill it in
    if !has_overlay_popup && !app.editor.visible {
        f.render_widget(ratatui::widgets::Clear, area);
        return;
    }

    // Overlay popup or editor is visible - render output with ratatui
    // (crossterm is skipped when popups are shown to avoid bleed-through)
    // Write directly to the ratatui buffer, bypassing Paragraph widget,
    // with manual ANSI parsing for proper color reproduction.

    // Fill the entire output area with background first
    f.render_widget(ratatui::widgets::Clear, area);

    // Build visual lines (wrapped ANSI strings) by working backwards from scroll_offset
    let mut wrapped_lines: Vec<String> = Vec::new();
    let new_line_indicator = app.settings.new_line_indicator;
    let nli_prefix_width: usize = 2; // "▶ " = 2 columns

    if !world.output_lines.is_empty() {
        let end_line = world.scroll_offset.min(world.output_lines.len().saturating_sub(1));

        // Cache "now" for timestamp formatting
        let cached_now = CachedNow::new();

        for line_idx in (0..=end_line).rev() {
            let line = &world.output_lines[line_idx];
            let is_new = new_line_indicator && line.marked_new;

            let expanded = match process_output_line(line, app.show_tags, app.settings.temp_convert_enabled, app.settings.zwj_enabled, &cached_now) {
                Some(text) if text.is_empty() => {
                    let prefix = if is_new { "\x1b[32m▶\x1b[0m ".to_string() } else { String::new() };
                    wrapped_lines.insert(0, prefix);
                    if wrapped_lines.len() >= visible_height {
                        break;
                    }
                    continue;
                }
                Some(text) => text,
                None => continue,
            };

            // Wrap the line to fit the output area width (narrower if NLI prefix)
            let wrap_width = if is_new { area_width.saturating_sub(nli_prefix_width) } else { area_width };
            let wrapped = wrap_ansi_line(&expanded, wrap_width);

            for w in wrapped.into_iter().rev() {
                let prefixed = if is_new {
                    format!("\x1b[32m▶\x1b[0m {}", w)
                } else {
                    w
                };
                wrapped_lines.insert(0, prefixed);
            }

            if wrapped_lines.len() >= visible_height {
                break;
            }
        }
    }

    // Trim to visible_height from the bottom (keep the most recent lines)
    if wrapped_lines.len() > visible_height {
        let excess = wrapped_lines.len() - visible_height;
        wrapped_lines.drain(0..excess);
    }

    // Write each line directly to the ratatui buffer with ANSI-parsed styles
    let buf = f.buffer_mut();
    for (row_idx, line) in wrapped_lines.iter().enumerate() {
        if row_idx >= visible_height {
            break;
        }
        let y = area.y + row_idx as u16;
        ansi_string_to_buffer(buf, area.x, y, line, area.width);
    }
}

/// Render the split-screen editor panel
pub(crate) fn render_editor_panel(f: &mut Frame, app: &App, area: Rect) {
    let theme = app.settings.theme;

    // Get editor title with world name if editing notes
    let world_name = app.editor.world_index.map(|idx| app.worlds[idx].name.as_str());
    let title = app.editor.title(world_name);

    // Add focus indicator to title
    let title_with_focus = if app.editor.focus == EditorFocus::Editor {
        format!("[Ctrl+Space] {}", title)
    } else {
        format!(" {} ", title)
    };

    // Create bordered block - highlight border when focused
    let border_style = if app.editor.focus == EditorFocus::Editor {
        Style::default().fg(theme.fg_highlight())
    } else {
        Style::default().fg(theme.fg_dim())
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(title_with_focus)
        .style(Style::default().bg(theme.bg()));

    // Calculate inner area (accounting for border and button row)
    let inner = block.inner(area);
    let button_height = 1;
    let content_height = inner.height.saturating_sub(button_height);
    let content_width = inner.width as usize;

    // Split inner area into content and button row
    let content_area = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: content_height,
    };
    let button_area = Rect {
        x: inner.x,
        y: inner.y + content_height,
        width: inner.width,
        height: button_height,
    };

    // Clear the entire area first to prevent any bleed-through
    f.render_widget(ratatui::widgets::Clear, area);

    // Fill the entire area with background color
    let background = Block::default().style(Style::default().bg(theme.bg()));
    f.render_widget(background, area);

    // Render the border on top
    f.render_widget(block, area);

    // Get logical lines from buffer
    let logical_lines: Vec<&str> = app.editor.lines();
    let visible_lines = content_height as usize;
    let scroll_offset = app.editor.scroll_offset;

    // Wrap lines by character (letter wrapping) and track cursor position
    // Each visual line is (text, Option<cursor_col>) where cursor_col is Some if cursor is on this visual line
    let mut visual_lines: Vec<(String, Option<usize>)> = Vec::new();

    let cursor_line = app.editor.cursor_line;
    let cursor_col = app.editor.cursor_col;
    let show_cursor = app.editor.focus == EditorFocus::Editor;

    for (line_idx, line) in logical_lines.iter().enumerate() {
        let is_cursor_line = line_idx == cursor_line;
        let chars: Vec<char> = line.chars().collect();

        if chars.is_empty() {
            // Empty line - just add it, with cursor if applicable
            if is_cursor_line && show_cursor {
                visual_lines.push((String::new(), Some(0)));
            } else {
                visual_lines.push((String::new(), None));
            }
        } else if content_width == 0 {
            // Edge case: no width
            visual_lines.push((String::new(), None));
        } else {
            // Wrap the line by characters
            let mut pos = 0;
            while pos < chars.len() {
                let end = (pos + content_width).min(chars.len());
                let segment: String = chars[pos..end].iter().collect();

                // Check if cursor is in this segment
                let cursor_in_segment = if is_cursor_line && show_cursor {
                    if cursor_col >= pos && cursor_col <= end {
                        Some(cursor_col - pos)
                    } else {
                        None
                    }
                } else {
                    None
                };

                visual_lines.push((segment, cursor_in_segment));
                pos = end;
            }

            // If cursor is at end of line and line length is exact multiple of width,
            // we need an extra visual line for the cursor
            if is_cursor_line && show_cursor && cursor_col == chars.len() && chars.len() % content_width == 0 && !chars.is_empty() {
                visual_lines.push((String::new(), Some(0)));
            }
        }
    }

    // Build display lines from scroll_offset
    let mut display_lines: Vec<Line<'_>> = Vec::with_capacity(visible_lines);

    for (text, cursor_pos) in visual_lines.iter().skip(scroll_offset).take(visible_lines) {
        if let Some(col) = cursor_pos {
            // This visual line has the cursor
            let chars: Vec<char> = text.chars().collect();
            let col = (*col).min(chars.len());
            let before: String = chars[..col].iter().collect();
            let after: String = chars[col..].iter().collect();

            let spans = vec![
                Span::raw(before),
                Span::styled("│", Style::default().fg(theme.fg_highlight())),
                Span::raw(after),
            ];
            display_lines.push(Line::from(spans));
        } else {
            display_lines.push(Line::raw(text.clone()));
        }
    }

    // Fill remaining lines if content is shorter than visible area
    while display_lines.len() < visible_lines {
        display_lines.push(Line::raw("~"));
    }

    let content = Text::from(display_lines);
    let content_paragraph = Paragraph::new(content)
        .style(Style::default().bg(theme.bg()).fg(theme.fg()));
    f.render_widget(content_paragraph, content_area);

    // Render button row with proper background
    let save_style = Style::default().fg(theme.fg_success());
    let cancel_style = Style::default().fg(theme.fg_error());
    let button_text = Line::from(vec![
        Span::raw(" "),
        Span::styled("[S]", save_style),
        Span::raw("ave "),
        Span::styled("[Esc]", cancel_style),
        Span::raw(" Cancel"),
    ]);
    let button_paragraph = Paragraph::new(button_text)
        .style(Style::default().bg(theme.bg()).fg(theme.fg()));
    f.render_widget(button_paragraph, button_area);
}

pub(crate) fn render_splash_centered<'a>(world: &World, visible_height: usize, area_width: usize) -> Text<'a> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    // Calculate visible width of a string (excluding ANSI escape sequences)
    fn visible_width(s: &str) -> usize {
        let mut width = 0;
        let mut in_escape = false;
        for c in s.chars() {
            if c == '\x1b' {
                in_escape = true;
            } else if in_escape {
                if c == 'm' {
                    in_escape = false;
                }
            } else {
                width += unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
            }
        }
        width
    }

    // Calculate vertical centering - how many blank lines to add at top
    let content_height = world.output_lines.len();
    let vertical_padding = if visible_height > content_height {
        (visible_height - content_height) / 2
    } else {
        0
    };

    // Add vertical padding
    for _ in 0..vertical_padding {
        lines.push(Line::from(""));
    }

    // Center all lines while splash is showing
    for line in &world.output_lines {
        let line_width = visible_width(&line.text);
        let padding = if area_width > line_width {
            (area_width - line_width) / 2
        } else {
            0
        };

        // Create padded line
        let padded = if padding > 0 {
            format!("{:width$}{}", "", line.text, width = padding)
        } else {
            line.text.clone()
        };

        // Parse ANSI codes and convert to ratatui spans
        match ansi_to_tui::IntoText::into_text(&padded) {
            Ok(text) => {
                for l in text.lines {
                    lines.push(l);
                }
            }
            Err(_) => {
                lines.push(Line::raw(padded));
            }
        }
    }

    Text::from(lines)
}

pub(crate) fn format_more_count(count: usize) -> String {
    // Returns exactly 4 characters to make "More XXXX" or "Hist XXXX" = 9 chars total (right-justified)
    if count <= 9999 {
        format!("{:>4}", count)
    } else if count < 1_000_000 {
        // 10K, 99K, 100K, 999K etc.
        format!("{:>3}K", (count / 1000).min(999))
    } else {
        "Alot".to_string()
    }
}

pub(crate) fn render_separator_bar(f: &mut Frame, app: &App, area: Rect) {
    let width = area.width as usize;
    let world = app.current_world();
    let theme = app.settings.theme;

    // Build bar components
    let time_str = get_current_time_12hr();

    // Status indicator - always reserve space for "More XXXX" or "Hist XXXX" (9 chars)
    // Priority: Hist (when scrolled back) > More (when paused) > underscores
    const STATUS_INDICATOR_LEN: usize = 9;
    let (status_str, status_active) = if !world.is_at_bottom() {
        // Show History indicator when scrolled back (takes precedence over More)
        let lines_back = world.lines_from_bottom(app.show_tags);
        (format!("Hist {}", format_more_count(lines_back)), true)
    } else if world.paused && (!world.pending_lines.is_empty() || world.pending_count > 0) {
        // Show More indicator when paused with pending lines
        // Use pending_count for console mode (synced from daemon), pending_lines.len() for daemon mode
        let count = if !world.pending_lines.is_empty() { world.pending_lines.len() } else { world.pending_count };
        (format!("More {}", format_more_count(count)), true)
    } else {
        // Fill with underscores when nothing to show
        ("_".repeat(STATUS_INDICATOR_LEN), false)
    };

    // World name
    let world_display = world.name.clone();

    // Tag indicator (only shown when F2 toggled to show tags)
    let tag_indicator = if app.show_tags { " [tag]" } else { "" };

    // GMCP indicator (only shown when F9 toggled to enable GMCP processing)
    let gmcp_indicator = if world.gmcp_user_enabled { " [g]" } else { "" };

    // Activity indicator - positioned at column 24
    const ACTIVITY_POSITION: usize = 24;
    // In remote client mode, use the server's activity count
    let activity_count = if app.is_master {
        app.activity_count()
    } else {
        app.server_activity_count
    };

    // Determine activity string based on available space
    // Full format: "(Activity: X)", Short format: "(Act X)"
    let activity_str = if activity_count > 0 {
        let full_format = format!("(Activity: {})", activity_count);
        let short_format = format!("(Act {})", activity_count);
        // Use short format if screen is narrow (less than 60 chars)
        if width < 60 {
            short_format
        } else {
            full_format
        }
    } else {
        String::new()
    };

    // Time on the right (no space before it, underscores fill to it)
    let time_display = time_str.clone();

    // Create styled spans
    let mut spans = Vec::new();

    // Status indicator on the left (black on red if active, dim underscores if not)
    spans.push(Span::styled(
        status_str.clone(),
        if status_active {
            Style::default().fg(theme.button_selected_fg()).bg(theme.fg_error())
        } else {
            Style::default().fg(theme.fg_dim())
        },
    ));

    // Only show connection ball, world name, and tag indicator if world has ever connected
    // Use world.was_connected flag (set to true on first connection)
    let is_connected = world.connected;
    let was_connected = world.was_connected;

    let current_pos = if was_connected {
        // Connection status ball (green when connected, red when disconnected)
        spans.push(Span::styled(
            "● ",
            Style::default().fg(if is_connected { theme.fg_success() } else { theme.fg_error() }),
        ));

        // World name
        spans.push(Span::styled(
            world_display.clone(),
            Style::default().fg(theme.fg()),
        ));

        // Tag indicator (cyan, like prompt)
        if !tag_indicator.is_empty() {
            spans.push(Span::styled(
                tag_indicator.to_string(),
                Style::default().fg(theme.fg_accent()),
            ));
        }

        // GMCP indicator (cyan, like prompt)
        if !gmcp_indicator.is_empty() {
            spans.push(Span::styled(
                gmcp_indicator.to_string(),
                Style::default().fg(theme.fg_accent()),
            ));
        }

        // Calculate position: status + ball (2 chars) + world name + tag indicator + gmcp indicator
        status_str.len() + 2 + world_display.len() + tag_indicator.len() + gmcp_indicator.len()
    } else {
        // World never connected - don't show ball or name
        status_str.len()
    };

    // Add underscores to reach position 24 (or as close as possible)
    if !activity_str.is_empty() && current_pos < ACTIVITY_POSITION {
        let padding = ACTIVITY_POSITION - current_pos;
        spans.push(Span::styled(
            "_".repeat(padding),
            Style::default().fg(theme.fg_dim()),
        ));
    }

    // Activity indicator (highlight color)
    if !activity_str.is_empty() {
        spans.push(Span::styled(
            activity_str.clone(),
            Style::default()
                .fg(theme.fg_highlight())
                .add_modifier(Modifier::BOLD),
        ));
    }

    // Calculate underscore padding - fill between content and time
    let used_len = if activity_str.is_empty() {
        current_pos
    } else {
        ACTIVITY_POSITION.max(current_pos) + activity_str.len()
    };
    // Subtract 2 for the fixed underscores before time
    let underscore_count = width.saturating_sub(used_len + time_display.len() + 2);

    spans.push(Span::styled(
        "_".repeat(underscore_count),
        Style::default().fg(theme.fg_dim()),
    ));

    // Underscore separator before time (2 chars for extra spacing)
    spans.push(Span::styled("__", Style::default().fg(theme.fg_dim())));

    // Time on the right (no spaces around it)
    spans.push(Span::styled(time_display, Style::default().fg(theme.fg())));

    let line = Line::from(spans);
    let paragraph = Paragraph::new(line).style(Style::default().bg(theme.bg()));

    f.render_widget(paragraph, area);
}

pub(crate) fn render_input_area(f: &mut Frame, app: &mut App, area: Rect) {
    // Get prompt for current world only (clone to avoid borrow conflict)
    let prompt = app.current_world().prompt.clone();
    // Use visible length (without ANSI codes) for cursor positioning
    let prompt_len = strip_ansi_codes(&prompt).chars().count();

    let input_text = render_input(app, area.width as usize, &prompt);

    let input_paragraph = Paragraph::new(input_text);
    f.render_widget(input_paragraph, area);

    // Set cursor position (offset by prompt length on first line)
    let cursor_line = app.input.cursor_line();
    let viewport_line = cursor_line.saturating_sub(app.input.viewport_start_line);

    if viewport_line < app.input_height as usize {
        let inner_width = area.width.max(1) as usize;
        let first_line_capacity = inner_width.saturating_sub(prompt_len);
        let text_before_cursor = &app.input.buffer[..app.input.cursor_position];

        let mut col_width = 0usize;
        let mut is_first_line = true;

        for c in text_before_cursor.chars() {
            if c == '\n' {
                col_width = 0;
                is_first_line = false;
                continue;
            }
            let cw = display_width(&c.to_string());
            let capacity = if is_first_line { first_line_capacity } else { inner_width };
            col_width += cw;
            if capacity > 0 && col_width >= capacity {
                if col_width == capacity {
                    col_width = 0;
                } else {
                    col_width = cw;
                }
                is_first_line = false;
            }
        }

        let cursor_col = if is_first_line && app.input.viewport_start_line == 0 {
            col_width + prompt_len
        } else {
            col_width
        };

        let cursor_x = area.x + cursor_col as u16;
        // Calculate visual line within viewport
        let cursor_y = area.y + viewport_line as u16;
        f.set_cursor(cursor_x, cursor_y.min(area.y + area.height - 1));
    }
}

/// Like chars_for_display_width but also breaks at '\n'.
/// Returns (chars_count, has_newline). If has_newline is true, the caller should
/// skip the newline character after advancing by chars_count.
pub(crate) fn chars_for_line(chars: &[char], width: usize) -> (usize, bool) {
    // Check for newline before the width-based wrap point
    let (wrap_count, _) = chars_for_display_width(chars, width);
    if let Some(nl_pos) = chars[..wrap_count].iter().position(|&c| c == '\n') {
        (nl_pos, true)
    } else {
        (wrap_count, false)
    }
}

pub(crate) fn render_input(app: &mut App, width: usize, prompt: &str) -> Text<'static> {
    let tc = app.settings.theme;
    let misspelled = app.find_misspelled_words();
    let chars: Vec<char> = app.input.buffer.chars().collect();

    // Calculate visible prompt length (without ANSI codes)
    let prompt_visible_len = strip_ansi_codes(prompt).chars().count();

    if width == 0 {
        return Text::default();
    }

    let mut lines: Vec<Line<'static>> = Vec::new();

    // If we're at the start, render the prompt first
    if app.input.viewport_start_line == 0 && !prompt.is_empty() {
        // Check if prompt has ANSI codes
        let has_ansi = prompt.contains("\x1b[");

        if has_ansi {
            // Parse ANSI codes and render with proper styling
            match ansi_to_tui::IntoText::into_text(&prompt) {
                Ok(text) => {
                    // Collect all spans from all lines into a single line
                    let mut all_spans: Vec<Span<'static>> = Vec::new();
                    for line in text.lines {
                        for span in line.spans {
                            all_spans.push(span);
                        }
                    }
                    if !all_spans.is_empty() {
                        lines.push(Line::from(all_spans));
                    }
                }
                Err(_) => {
                    // Fallback: render as plain text without adding color
                    lines.push(Line::from(Span::raw(prompt.to_string())));
                }
            }
        } else {
            // No ANSI codes — render as-is without adding color
            lines.push(Line::from(Span::raw(prompt.to_string())));
        }
    }

    // Build a combined first line if prompt doesn't fill the width
    if app.input.viewport_start_line == 0 && !prompt.is_empty() && !lines.is_empty() {
        let prompt_line_chars = prompt_visible_len % width;
        let remaining_width = if prompt_line_chars == 0 && prompt_visible_len > 0 {
            0
        } else {
            width - prompt_line_chars
        };

        if remaining_width > 0 && !chars.is_empty() {
            // Add user input to the same line as prompt
            // Use display width to determine how many chars fit
            let (input_chars_on_first_line, first_line_has_nl) = chars_for_line(&chars, remaining_width);
            let first_input: String = chars[..input_chars_on_first_line].iter().collect();

            // Get the last line (which has the prompt) and append user input
            if let Some(last_line) = lines.last_mut() {
                let mut new_spans = last_line.spans.clone();
                // Check for misspellings in this portion
                let misspelled_in_range: Vec<_> = misspelled
                    .iter()
                    .filter(|(s, e)| *s < input_chars_on_first_line || *e <= input_chars_on_first_line)
                    .cloned()
                    .collect();

                if misspelled_in_range.is_empty() {
                    new_spans.push(Span::raw(first_input));
                } else {
                    // Handle misspellings
                    let mut pos = 0;
                    for (start, end) in &misspelled_in_range {
                        let s = (*start).min(input_chars_on_first_line);
                        let e = (*end).min(input_chars_on_first_line);
                        if pos < s {
                            let text: String = chars[pos..s].iter().collect();
                            new_spans.push(Span::raw(text));
                        }
                        if s < e {
                            let text: String = chars[s..e].iter().collect();
                            new_spans.push(Span::styled(text, Style::default().fg(tc.fg_error()).add_modifier(Modifier::BOLD)));
                        }
                        pos = e;
                    }
                    if pos < input_chars_on_first_line {
                        let text: String = chars[pos..input_chars_on_first_line].iter().collect();
                        new_spans.push(Span::raw(text));
                    }
                }
                *last_line = Line::from(new_spans);
            }

            // Now handle remaining input on subsequent lines
            let mut char_pos = input_chars_on_first_line;
            // Skip newline char from first line if present
            if first_line_has_nl && char_pos < chars.len() && chars[char_pos] == '\n' {
                char_pos += 1;
            }
            while char_pos < chars.len() && lines.len() < app.input_height as usize {
                let (chars_on_line, has_nl) = chars_for_line(&chars[char_pos..], width);
                let line_end = char_pos + chars_on_line;
                let mut spans: Vec<Span<'static>> = Vec::new();
                let mut current_pos = char_pos;

                while current_pos < line_end {
                    let in_misspelled = misspelled
                        .iter()
                        .find(|(s, e)| current_pos >= *s && current_pos < *e);

                    if let Some(&(word_start, word_end)) = in_misspelled {
                        if current_pos > char_pos && spans.is_empty() {
                            let before_end = word_start.min(line_end);
                            if before_end > char_pos {
                                let text: String = chars[char_pos..before_end].iter().collect();
                                spans.push(Span::raw(text));
                            }
                        }
                        let mis_start = word_start.max(char_pos);
                        let mis_end = word_end.min(line_end);
                        let text: String = chars[mis_start..mis_end].iter().collect();
                        spans.push(Span::styled(text, Style::default().fg(tc.fg_error()).add_modifier(Modifier::BOLD)));
                        current_pos = mis_end;
                    } else {
                        let next_mis = misspelled
                            .iter()
                            .filter(|(s, _)| *s > current_pos && *s < line_end)
                            .map(|(s, _)| *s)
                            .min();
                        let chunk_end = next_mis.unwrap_or(line_end);
                        let text: String = chars[current_pos..chunk_end].iter().collect();
                        spans.push(Span::raw(text));
                        current_pos = chunk_end;
                    }
                }

                if spans.is_empty() {
                    let text: String = chars[char_pos..line_end].iter().collect();
                    lines.push(Line::from(text));
                } else {
                    lines.push(Line::from(spans));
                }
                char_pos = line_end;
                // Skip the newline character
                if has_nl && char_pos < chars.len() && chars[char_pos] == '\n' {
                    char_pos += 1;
                }
            }
        } else if remaining_width == 0 {
            // Prompt fills the line exactly, user input starts on next line
            let mut char_pos = 0;
            while char_pos < chars.len() && lines.len() < app.input_height as usize {
                let (chars_on_line, has_nl) = chars_for_line(&chars[char_pos..], width);
                let line_end = char_pos + chars_on_line;
                let text: String = chars[char_pos..line_end].iter().collect();
                lines.push(Line::from(text));
                char_pos = line_end;
                if has_nl && char_pos < chars.len() && chars[char_pos] == '\n' {
                    char_pos += 1;
                }
            }
        }
    } else if app.input.viewport_start_line == 0 && prompt.is_empty() {
        // No prompt, just render user input
        let mut char_pos = 0;
        while char_pos < chars.len() && lines.len() < app.input_height as usize {
            let (chars_on_line, has_nl) = chars_for_line(&chars[char_pos..], width);
            let line_end = char_pos + chars_on_line;
            let mut spans: Vec<Span<'static>> = Vec::new();
            let mut current_pos = char_pos;

            while current_pos < line_end {
                let in_misspelled = misspelled
                    .iter()
                    .find(|(s, e)| current_pos >= *s && current_pos < *e);

                if let Some(&(word_start, word_end)) = in_misspelled {
                    if current_pos > char_pos && spans.is_empty() {
                        let before_end = word_start.min(line_end);
                        if before_end > char_pos {
                            let text: String = chars[char_pos..before_end].iter().collect();
                            spans.push(Span::raw(text));
                        }
                    }
                    let mis_start = word_start.max(char_pos);
                    let mis_end = word_end.min(line_end);
                    let text: String = chars[mis_start..mis_end].iter().collect();
                    spans.push(Span::styled(text, Style::default().fg(tc.fg_error()).add_modifier(Modifier::BOLD)));
                    current_pos = mis_end;
                } else {
                    let next_mis = misspelled
                        .iter()
                        .filter(|(s, _)| *s > current_pos && *s < line_end)
                        .map(|(s, _)| *s)
                        .min();
                    let chunk_end = next_mis.unwrap_or(line_end);
                    let text: String = chars[current_pos..chunk_end].iter().collect();
                    spans.push(Span::raw(text));
                    current_pos = chunk_end;
                }
            }

            if spans.is_empty() {
                let text: String = chars[char_pos..line_end].iter().collect();
                lines.push(Line::from(text));
            } else {
                lines.push(Line::from(spans));
            }
            char_pos = line_end;
            if has_nl && char_pos < chars.len() && chars[char_pos] == '\n' {
                char_pos += 1;
            }
        }
    } else {
        // Scrolled down, don't show prompt
        // Calculate start_char by iterating through lines to find correct starting position
        // This accounts for variable chars-per-line due to display width differences
        // IMPORTANT: First line has less capacity due to prompt
        let first_line_width = width.saturating_sub(prompt_visible_len);
        let mut start_char = 0;
        for line_idx in 0..app.input.viewport_start_line {
            if start_char >= chars.len() {
                break;
            }
            // First line has reduced width due to prompt, subsequent lines have full width
            let line_width = if line_idx == 0 { first_line_width } else { width };
            let (chars_on_line, has_nl) = chars_for_line(&chars[start_char..], line_width);
            start_char += chars_on_line.max(1); // Ensure progress even with weird chars
            if has_nl && start_char < chars.len() && chars[start_char] == '\n' {
                start_char += 1;
            }
        }
        let mut char_pos = start_char;
        while char_pos < chars.len() && lines.len() < app.input_height as usize {
            let (chars_on_line, has_nl) = chars_for_line(&chars[char_pos..], width);
            let line_end = char_pos + chars_on_line;
            let mut spans: Vec<Span<'static>> = Vec::new();
            let mut current_pos = char_pos;

            while current_pos < line_end {
                let in_misspelled = misspelled
                    .iter()
                    .find(|(s, e)| current_pos >= *s && current_pos < *e);

                if let Some(&(word_start, word_end)) = in_misspelled {
                    if current_pos > char_pos && spans.is_empty() {
                        let before_end = word_start.min(line_end);
                        if before_end > char_pos {
                            let text: String = chars[char_pos..before_end].iter().collect();
                            spans.push(Span::raw(text));
                        }
                    }
                    let mis_start = word_start.max(char_pos);
                    let mis_end = word_end.min(line_end);
                    let text: String = chars[mis_start..mis_end].iter().collect();
                    spans.push(Span::styled(text, Style::default().fg(tc.fg_error()).add_modifier(Modifier::BOLD)));
                    current_pos = mis_end;
                } else {
                    let next_mis = misspelled
                        .iter()
                        .filter(|(s, _)| *s > current_pos && *s < line_end)
                        .map(|(s, _)| *s)
                        .min();
                    let chunk_end = next_mis.unwrap_or(line_end);
                    let text: String = chars[current_pos..chunk_end].iter().collect();
                    spans.push(Span::raw(text));
                    current_pos = chunk_end;
                }
            }

            if spans.is_empty() {
                let text: String = chars[char_pos..line_end].iter().collect();
                lines.push(Line::from(text));
            } else {
                lines.push(Line::from(spans));
            }
            char_pos = line_end;
            if has_nl && char_pos < chars.len() && chars[char_pos] == '\n' {
                char_pos += 1;
            }
        }
    }

    // Pad remaining lines
    while lines.len() < app.input_height as usize {
        lines.push(Line::from(""));
    }

    Text::from(lines)
}

pub(crate) fn render_confirm_dialog(f: &mut Frame, app: &App) {
    if !app.confirm_dialog.visible {
        return;
    }

    let area = f.size();
    let dialog = &app.confirm_dialog;
    let theme = app.settings.theme;

    // Build button styles with background highlight
    let yes_style = if dialog.yes_selected {
        Style::default().fg(theme.button_selected_fg()).bg(theme.button_selected_bg()).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.fg())
    };
    let no_style = if !dialog.yes_selected {
        Style::default().fg(theme.button_selected_fg()).bg(theme.button_selected_bg()).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.fg())
    };

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(dialog.message.clone(), Style::default().fg(theme.fg()))).alignment(Alignment::Center),
        Line::from(""),
        Line::from(vec![
            Span::styled("[ Yes ]", yes_style),
            Span::raw("  "),
            Span::styled("[ No ]", no_style),
        ]).alignment(Alignment::Center),
    ];

    // Calculate dynamic size based on content
    let message_width = dialog.message.chars().count();
    let buttons_width = 17; // "[ Yes ]  [ No ]"
    let content_width = message_width.max(buttons_width);
    let popup_width = ((content_width + 6) as u16).min(area.width.saturating_sub(4)); // +6 for borders and padding
    let popup_height = ((lines.len() + 2) as u16).min(area.height.saturating_sub(2)); // +2 for borders

    let x = area.width.saturating_sub(popup_width) / 2;
    let y = area.height.saturating_sub(popup_height) / 2;

    let popup_area = Rect::new(x, y, popup_width, popup_height);

    // Clear the background
    f.render_widget(ratatui::widgets::Clear, popup_area);

    let popup_block = Block::default()
        .title(" Confirm ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.fg_error()))
        .style(Style::default().bg(theme.popup_bg()));

    let popup_text = Paragraph::new(lines).block(popup_block);

    f.render_widget(popup_text, popup_area);
}

pub(crate) fn render_filter_popup(f: &mut Frame, app: &App) {
    if !app.filter_popup.visible {
        return;
    }

    let area = f.size();
    let filter = &app.filter_popup;
    let theme = app.settings.theme;

    // Small popup in upper right corner
    let popup_width = 40u16.min(area.width);
    let popup_height = 3u16;

    let x = area.width.saturating_sub(popup_width); // Right edge
    let y = 0; // Top edge, no gap

    let popup_area = Rect::new(x, y, popup_width, popup_height);

    // Clear the background
    f.render_widget(ratatui::widgets::Clear, popup_area);

    // Show filter text with cursor
    let mut display_text = filter.filter_text.clone();
    display_text.insert(filter.cursor, '|');

    let lines = vec![
        Line::from(vec![
            Span::styled("Filter: ", Style::default().fg(theme.fg_accent())),
            Span::styled(display_text, Style::default().fg(theme.fg())),
        ]),
    ];

    let popup_block = Block::default()
        .title(" Find [Esc to close] ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.popup_border()))
        .style(Style::default().bg(theme.popup_bg()));

    let popup_text = Paragraph::new(lines).block(popup_block);

    f.render_widget(popup_text, popup_area);
}

/// Render new unified popup system
pub(crate) fn render_new_popup(f: &mut Frame, app: &mut App) {
    let tc = app.settings.theme;
    if let Some(state) = app.popup_manager.current_mut() {
        popup::console_renderer::render_popup(f, state, &tc);
    }
}
