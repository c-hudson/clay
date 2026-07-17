//! Console renderer for popups using ratatui
//!
//! Renders PopupState to the terminal using ratatui Frame.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use super::{
    ButtonStyle, ContentArea, ElementSelection, FieldKind, PopupLayout, PopupState,
};
use crate::encoding::Theme;

/// Render a popup to the console
pub fn render_popup(f: &mut Frame, state: &mut PopupState, theme: &Theme) {
    if !state.visible {
        return;
    }

    let area = f.size();
    let layout = &state.definition.layout;

    // Calculate popup dimensions
    let (popup_area, inner_area) = calculate_popup_area(area, layout, state);

    // Update actual_content_height for scroll calculations
    // Inner area height minus space for buttons (if any)
    let button_space = if state.definition.buttons.is_empty() { 0 } else { 2 };
    state.actual_content_height = Some((inner_area.height as usize).saturating_sub(button_space));

    // Clear background using Clear widget
    f.render_widget(Clear, popup_area);

    // Fill the ENTIRE popup area with background color (including where border will go)
    // This ensures no bleed-through from previous content
    let fill_text_full = " ".repeat(popup_area.width as usize);
    for row in 0..popup_area.height {
        let row_area = Rect::new(popup_area.x, popup_area.y + row, popup_area.width, 1);
        let fill_span = Span::styled(&fill_text_full, Style::default().bg(theme.popup_bg()));
        f.render_widget(Paragraph::new(Line::from(fill_span)), row_area);
    }

    // Create border block with title
    let title = format!(" {} ", state.definition.title);
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.popup_border()))
        .style(Style::default().bg(theme.popup_bg()));

    f.render_widget(block, popup_area);

    // Render content inside the block
    render_popup_content(f, state, inner_area, theme);
}

/// Calculate popup area based on layout settings
fn calculate_popup_area(area: Rect, layout: &PopupLayout, state: &PopupState) -> (Rect, Rect) {
    // Calculate width
    let max_width = if layout.max_width_percent > 0 {
        (area.width as usize * layout.max_width_percent / 100) as u16
    } else {
        area.width.saturating_sub(4)
    };

    let popup_width = (layout.min_width as u16)
        .max(calculate_content_width(state, layout) as u16)
        .min(max_width)
        .min(area.width.saturating_sub(2));

    // Calculate height based on content
    let content_height = calculate_content_height(state);

    // For non-centered vertical popups, available space is from line 2 (y=1) to separator bar
    // Must not overlap input window (separator + input = 3 lines from bottom)
    let max_height = if layout.center_vertical {
        area.height.saturating_sub(2)
    } else {
        // Available from y=1, leaving 3 lines at bottom (separator + 2 input lines)
        area.height.saturating_sub(4)
    };

    let popup_height = (content_height as u16 + 2) // +2 for borders
        .min(max_height)
        .max(5);

    // Calculate position
    let x = if layout.anchor_bottom_left {
        layout.anchor_x.min(area.width.saturating_sub(popup_width))
    } else if layout.center_horizontal {
        area.width.saturating_sub(popup_width) / 2
    } else {
        // Position in upper right for non-centered popups (like filter)
        area.width.saturating_sub(popup_width)
    };

    // For vertical positioning:
    // - anchor_bottom_left: position just above separator bar
    // - center_vertical=true: center in the full area
    // - center_vertical=false: center between line 2 (y=1) and above input window
    let y = if layout.anchor_bottom_left {
        // Place popup so its bottom edge is just above the separator bar
        // Separator is at area.height - 3 (separator + 2 input lines)
        area.height.saturating_sub(popup_height).saturating_sub(3)
    } else if layout.center_vertical {
        area.height.saturating_sub(popup_height) / 2
    } else {
        // Center between y=1 (line 2) and max position (leave 3 lines for separator + input)
        let min_y = 1u16;
        let max_y = area.height.saturating_sub(popup_height).saturating_sub(3);
        if max_y <= min_y {
            min_y
        } else {
            min_y + (max_y - min_y) / 2
        }
    };

    let popup_area = Rect::new(x, y, popup_width, popup_height);

    // Inner area (inside borders)
    let inner_area = Rect::new(
        popup_area.x + 1,
        popup_area.y + 1,
        popup_area.width.saturating_sub(2),
        popup_area.height.saturating_sub(2),
    );

    (popup_area, inner_area)
}

/// Calculate required content width
fn calculate_content_width(state: &PopupState, layout: &PopupLayout) -> usize {
    let mut max_width = layout.min_width;

    for field in &state.definition.fields {
        if !field.visible {
            continue;
        }

        let field_width = match &field.kind {
            FieldKind::Text { value, .. } => {
                layout.label_width + value.len() + 4
            }
            FieldKind::Label { text } => {
                // Find the longest line in the label
                text.lines().map(|l| l.len()).max().unwrap_or(0) + 2
            }
            FieldKind::List { items, .. } => {
                items.iter()
                    .flat_map(|i| &i.columns)
                    .map(|c| c.len())
                    .max()
                    .unwrap_or(20) + 4
            }
            FieldKind::ScrollableContent { lines, .. } => {
                // Find the longest line in the scrollable content
                lines.iter().map(|l| l.len()).max().unwrap_or(40) + 4
            }
            FieldKind::EditableList { items, .. } => {
                // Width driven by label + min edit space; items scroll horizontally
                let label_part = layout.label_width;
                let item_preview = items.iter().map(|s| s.len().min(40)).max().unwrap_or(20);
                label_part + item_preview + 4
            }
            _ => layout.label_width + 20,
        };

        max_width = max_width.max(field_width);
    }

    max_width
}

/// Calculate required content height
fn calculate_content_height(state: &PopupState) -> usize {
    let mut height = 0;
    let layout = &state.definition.layout;

    for field in &state.definition.fields {
        if !field.visible {
            continue;
        }

        // Add blank line before list if configured
        if layout.blank_line_before_list {
            if let FieldKind::List { .. } = &field.kind {
                height += 1;
            }
        }

        height += match &field.kind {
            FieldKind::Separator => 1,
            FieldKind::Label { text } => text.lines().count().max(1),
            FieldKind::MultilineText { visible_lines, .. } => *visible_lines,
            FieldKind::List { visible_height, headers, .. } => {
                // Use visible_height (set at popup creation) to maintain consistent size
                // This prevents the popup from shrinking when filtering
                let list_height = *visible_height;
                // Add 1 for header row if headers are present
                if headers.is_some() {
                    list_height + 1
                } else {
                    list_height
                }
            }
            FieldKind::ScrollableContent { visible_height, .. } => {
                *visible_height
            }
            FieldKind::EditableList { visible_height, .. } => {
                *visible_height
            }
            _ => 1,
        };
    }

    // Add button row if there are buttons
    if !state.definition.buttons.is_empty() {
        height += 2; // One blank line + button row
    }

    height
}

/// One measured row in the popup's top-level field list — computed once up front so the
/// scroll-window decision (which whole fields are currently visible) and the actual
/// rendering agree exactly. `row_start`/`height` are in the same relative-row units as
/// `PopupState.scroll_offset` (0 = first row of the field area).
#[derive(Debug, Clone, PartialEq)]
struct FieldRow {
    index: usize,
    height: usize,
    row_start: usize,
}

/// Pure, testable core of the popup field-list scroll/windowing decision — measures every
/// visible field's height/row-start, decides whether the field list needs to scroll to fit
/// `area_height`, and (if so) adjusts `scroll_offset` to keep the selected field in view.
/// Takes no ratatui types (unlike `render_popup_content`, which wraps this with the actual
/// `Frame` drawing) so it can be unit-tested directly — mirrors how `build_display_lines` in
/// rendering.rs separates pure layout logic from its Frame-coupled caller.
///
/// Returns `(rows, total_field_rows, needs_scroll, new_scroll_offset)`.
fn compute_field_layout(
    definition: &super::PopupDefinition,
    selected: &ElementSelection,
    scroll_offset: usize,
    area_height: usize,
    has_buttons: bool,
) -> (Vec<FieldRow>, usize, bool, usize) {
    let blank_line_before_list = definition.layout.blank_line_before_list;
    // Reserve space for buttons if present (1 blank line + 1 button row) — the caller pins
    // these to the bottom of the popup regardless of how many fields there are or how the
    // field list scrolls (previously the button row was drawn at a running cursor that
    // could run past the bottom of a short terminal, hiding the buttons entirely — see plan
    // `the-android-app-is-steady-sphinx.md`).
    let button_space = if has_buttons { 2 } else { 0 };
    let visible_rows = area_height.saturating_sub(button_space);

    // --- Measure every visible field's height and cumulative row offset. Mirrors the
    // per-kind height logic that used to run inline in the render loop, unchanged, so
    // List/ScrollableContent/EditableList fields keep self-sizing against whatever's left
    // in the box exactly as before; only the windowing decision (which whole fields to
    // actually draw) is new.
    let mut rows: Vec<FieldRow> = Vec::with_capacity(definition.fields.len());
    let mut cursor = 0usize;
    for (i, field) in definition.fields.iter().enumerate() {
        if !field.visible {
            continue;
        }
        if blank_line_before_list {
            if let FieldKind::List { .. } = &field.kind {
                cursor += 1;
            }
        }
        let remaining_height = area_height.saturating_sub(cursor);
        let available_for_field = remaining_height.saturating_sub(button_space);
        let field_height = match &field.kind {
            FieldKind::Label { text } => text.lines().count().max(1),
            FieldKind::MultilineText { visible_lines, .. } => *visible_lines,
            FieldKind::List { visible_height, headers, .. } => {
                // Use visible_height to maintain consistent size (don't shrink when filtering)
                let header_rows = if headers.is_some() { 1 } else { 0 };
                let max_items = available_for_field.saturating_sub(header_rows);
                (*visible_height).min(max_items) + header_rows
            }
            FieldKind::ScrollableContent { visible_height, .. } => {
                (*visible_height).min(available_for_field)
            }
            FieldKind::EditableList { visible_height, .. } => {
                (*visible_height).min(available_for_field)
            }
            _ => 1,
        };
        rows.push(FieldRow { index: i, height: field_height, row_start: cursor });
        cursor += field_height;
    }
    let total_field_rows = rows.last().map(|r| r.row_start + r.height).unwrap_or(0);
    let needs_scroll = total_field_rows > visible_rows;

    // --- Scroll offset: keep the currently selected field fully in view, then clamp so we
    // never scroll past the end. Called at render time (every frame, from the current
    // terminal size and selection) rather than patching every place selection can change
    // (Tab/Shift+Tab, Up/Down, shortcut keys, mouse clicks all funnel through different
    // PopupState methods) — mirrors how render_list_field clamps its own scroll fresh every
    // frame.
    let new_scroll_offset = if needs_scroll {
        let mut offset = scroll_offset;
        if let ElementSelection::Field(sel_id) = selected {
            if let Some(sel_row) = rows.iter().find(|r| definition.fields[r.index].id == *sel_id) {
                if sel_row.row_start < offset {
                    offset = sel_row.row_start;
                } else if sel_row.row_start + sel_row.height > offset + visible_rows {
                    offset = sel_row.row_start + sel_row.height - visible_rows;
                }
            }
        }
        offset.min(total_field_rows.saturating_sub(visible_rows))
    } else {
        0
    };

    (rows, total_field_rows, needs_scroll, new_scroll_offset)
}

/// Render popup content (fields and buttons)
fn render_popup_content(f: &mut Frame, state: &mut PopupState, area: Rect, theme: &Theme) {
    let available_width = area.width as usize;
    let has_buttons = !state.definition.buttons.is_empty();
    let button_space = if has_buttons { 2 } else { 0 };
    let visible_rows = (area.height as usize).saturating_sub(button_space);

    let (rows, total_field_rows, needs_scroll, new_scroll_offset) = compute_field_layout(
        &state.definition,
        &state.selected,
        state.scroll_offset,
        area.height as usize,
        has_buttons,
    );
    state.scroll_offset = new_scroll_offset;
    state.field_list_needs_scroll = needs_scroll;
    let scroll_offset = state.scroll_offset;

    // Reserve a column on the right for the scrollbar (only when actually scrolling), same
    // as render_list_field's content_width calculation.
    let scrollbar_width: u16 = if needs_scroll { 1 } else { 0 };
    let field_area_width = area.width.saturating_sub(scrollbar_width);

    // Clear hit areas and content areas for this render pass
    state.hit_areas.clear();
    state.content_areas.clear();

    let mut field_hit_areas: Vec<(Rect, super::FieldId)> = Vec::new();
    let mut content_area_infos: Vec<ContentArea> = Vec::new();

    for row in &rows {
        // Skip whole fields entirely above or below the visible window — same granularity
        // List uses for its own items (never splits a row mid-field).
        if row.row_start + row.height <= scroll_offset {
            continue;
        }
        if row.row_start >= scroll_offset + visible_rows {
            break;
        }

        let y = area.y + (row.row_start - scroll_offset) as u16;
        let field_area = Rect::new(area.x, y, field_area_width, row.height as u16);
        let field_id = state.definition.fields[row.index].id;
        let is_selected = matches!(&state.selected, ElementSelection::Field(id) if *id == field_id);

        // Collect content area info for mouse highlighting
        match &state.definition.fields[row.index].kind {
            FieldKind::ScrollableContent { lines, scroll_offset, .. } => {
                content_area_infos.push(ContentArea {
                    area: field_area,
                    field_id,
                    scroll_offset: *scroll_offset,
                    total_lines: lines.len(),
                });
            }
            FieldKind::List { items, headers, scroll_offset, .. } => {
                let header_rows = if headers.is_some() { 1 } else { 0 };
                let list_area = Rect::new(
                    field_area.x,
                    field_area.y + header_rows as u16,
                    field_area.width,
                    field_area.height.saturating_sub(header_rows as u16),
                );
                content_area_infos.push(ContentArea {
                    area: list_area,
                    field_id,
                    scroll_offset: *scroll_offset,
                    total_lines: items.len(),
                });
            }
            _ => {}
        }

        render_field(f, state, &state.definition.fields[row.index], field_area, is_selected, theme);

        field_hit_areas.push((field_area, field_id));
    }
    // Record field hit areas for mouse click detection
    for (area, id) in field_hit_areas {
        state.hit_areas.push((area, ElementSelection::Field(id)));
    }
    // Record content areas for mouse highlighting
    state.content_areas = content_area_infos;

    // Field-list scrollbar (mirrors render_list_field's visual style via the shared helper).
    if needs_scroll {
        let scrollbar_x = area.x + area.width.saturating_sub(1);
        render_scrollbar_column(
            f, scrollbar_x, area.y, area.y + visible_rows as u16,
            visible_rows, total_field_rows, scroll_offset, theme,
        );
    }

    // Buttons pinned to the bottom of `area`, independent of field scrolling — always
    // inside the popup box since `area.height` is already clamped to the terminal
    // (calculate_popup_area) and popup_height is guaranteed >= 5, so inner area.height >= 3.
    if has_buttons {
        let blank_y = area.y + area.height.saturating_sub(2);
        let blank_area = Rect::new(area.x, blank_y, area.width, 1);
        let blank_line = Line::from(Span::styled(
            " ".repeat(area.width as usize),
            Style::default().bg(theme.popup_bg()),
        ));
        f.render_widget(Paragraph::new(blank_line), blank_area);

        let button_y = area.y + area.height.saturating_sub(1);
        let button_area = Rect::new(area.x, button_y, area.width, 1);
        render_buttons(f, state, button_area, theme);
    }

    // Render error message if present
    if let Some(error) = &state.error {
        let error_y = area.y + area.height.saturating_sub(1);
        let error_area = Rect::new(area.x, error_y, area.width, 1);
        let error_line = Line::from(Span::styled(
            truncate_str(error, available_width),
            Style::default().fg(theme.fg_error()),
        ));
        f.render_widget(Paragraph::new(error_line), error_area);
    }
}

/// Render a single field
fn render_field(
    f: &mut Frame,
    state: &PopupState,
    field: &super::Field,
    area: Rect,
    is_selected: bool,
    theme: &Theme,
) {
    let layout = &state.definition.layout;
    let label_width = layout.label_width;

    match &field.kind {
        FieldKind::Separator => {
            let line = Line::from(Span::styled(
                "─".repeat(area.width as usize),
                Style::default().fg(theme.fg_dim()),
            ));
            f.render_widget(Paragraph::new(line), area);
        }

        FieldKind::Label { text } => {
            let lines: Vec<Line> = text
                .lines()
                .map(|l| Line::from(Span::styled(l.to_string(), Style::default().fg(theme.fg()))))
                .collect();
            f.render_widget(Paragraph::new(lines), area);
        }

        FieldKind::Text { value, masked, placeholder } => {
            // Calculate available width for value area
            let padding_width = label_width.saturating_sub(field.label.len() + 2);
            let label_total = padding_width + field.label.len() + 2; // padding + label + ": "
            let value_area_width = (area.width as usize).saturating_sub(label_total).saturating_sub(1);

            let display_value = if is_selected {
                // Show cursor when selected (whether editing or not)
                // Use edit_buffer if editing, otherwise use field value
                let buf = if state.editing {
                    if *masked {
                        "*".repeat(state.edit_buffer.len())
                    } else {
                        state.edit_buffer.clone()
                    }
                } else if *masked {
                    "*".repeat(value.len())
                } else {
                    value.clone()
                };
                let chars: Vec<char> = buf.chars().collect();
                // When not editing, put cursor at end
                let cursor_pos = if state.editing {
                    state.edit_cursor.min(chars.len())
                } else {
                    chars.len()
                };

                // Calculate viewport scroll to keep cursor visible
                // Reserve 3 chars for cursor and potential < > indicators
                let visible_width = value_area_width.saturating_sub(3);
                let scroll = if visible_width == 0 || cursor_pos <= visible_width {
                    // Cursor fits from beginning
                    0
                } else {
                    // Keep cursor visible with some margin from right edge
                    let margin = (visible_width / 4).max(1);
                    cursor_pos.saturating_sub(visible_width - margin)
                };

                // Build visible portion with cursor
                let has_left = scroll > 0;
                let visible_start = scroll;
                let visible_end = (scroll + visible_width).min(chars.len());
                let has_right = visible_end < chars.len();

                let mut result = String::new();
                if has_left {
                    result.push('<');
                }

                // Render visible chars with cursor
                let visible_cursor = cursor_pos.saturating_sub(scroll);
                for (i, c) in chars.iter().enumerate().skip(visible_start).take(visible_end - visible_start) {
                    let idx = i - visible_start;
                    if idx == visible_cursor {
                        result.push('│');
                    }
                    result.push(*c);
                }
                // Cursor at end
                if visible_cursor >= visible_end - visible_start {
                    result.push('│');
                }

                if has_right {
                    result.push('>');
                }

                result
            } else if value.is_empty() {
                placeholder.clone().unwrap_or_default()
            } else if *masked {
                "*".repeat(value.len())
            } else {
                value.clone()
            };

            render_labeled_field_with_shortcut(f, &field.label, &display_value, area, label_width, is_selected, field.shortcut, theme);
        }

        FieldKind::Toggle { value } => {
            // Use checkmark on Unix/macOS, 'x' on Windows (Windows console has limited Unicode support)
            #[cfg(not(windows))]
            let display_value = if *value { "[✓]" } else { "[ ]" };
            #[cfg(windows)]
            let display_value = if *value { "[x]" } else { "[ ]" };
            render_labeled_field(f, &field.label, display_value, area, label_width, is_selected, theme);
        }

        FieldKind::Select { options, selected_index } => {
            let display_value = options
                .get(*selected_index)
                .map(|o| format!("[ {} ]", o.label))
                .unwrap_or_else(|| "[ - ]".to_string());
            render_labeled_field(f, &field.label, &display_value, area, label_width, is_selected, theme);
        }

        FieldKind::Number { value, .. } => {
            let display_value = format!("[ {} ]", value);
            render_labeled_field(f, &field.label, &display_value, area, label_width, is_selected, theme);
        }

        FieldKind::MultilineText { value, visible_lines, scroll_offset } => {
            // Use edit_buffer if this field is selected and being edited
            let display_value = if is_selected && state.editing {
                &state.edit_buffer
            } else {
                value
            };

            // Calculate label area similar to other labeled fields
            let padding_width = label_width.saturating_sub(field.label.len() + 2);
            let label_total = padding_width + field.label.len() + 2; // padding + label + ": "
            let value_start_x = area.x + label_total as u16;
            let value_width = area.width.saturating_sub(label_total as u16);
            let wrap_width = if value_width > 0 { value_width as usize } else { 1 };

            // Render label on first line
            let label_style = Style::default().fg(theme.fg_dim());
            let padding: String = " ".repeat(padding_width);
            let label_line = Line::from(vec![
                Span::styled(padding, label_style),
                Span::styled(format!("{}: ", field.label), label_style),
            ]);
            f.render_widget(Paragraph::new(label_line), Rect::new(area.x, area.y, label_total as u16, 1));

            // Wrap text into display lines and track cursor position
            // Each display line is a (text, has_cursor, cursor_col) tuple
            struct DisplayLine {
                text: String,
                cursor_col: Option<usize>, // Some(col) if cursor is on this line
            }

            let mut display_lines: Vec<DisplayLine> = Vec::new();
            let mut cursor_display_line: Option<usize> = None;

            // Calculate cursor position in the text
            let cursor_char_pos = if is_selected {
                if state.editing {
                    state.edit_cursor
                } else {
                    display_value.chars().count()
                }
            } else {
                usize::MAX
            };

            // Process each text line (separated by \n)
            let mut char_offset = 0;
            let text_lines: Vec<&str> = display_value.split('\n').collect();

            for (line_idx, text_line) in text_lines.iter().enumerate() {
                let line_chars: Vec<char> = text_line.chars().collect();
                let line_len = line_chars.len();

                if line_len == 0 {
                    // Empty line
                    let has_cursor = cursor_char_pos == char_offset;
                    if has_cursor {
                        cursor_display_line = Some(display_lines.len());
                    }
                    display_lines.push(DisplayLine {
                        text: String::new(),
                        cursor_col: if has_cursor { Some(0) } else { None },
                    });
                } else {
                    // Wrap line into chunks of wrap_width
                    let mut pos = 0;
                    while pos < line_len {
                        let end = (pos + wrap_width).min(line_len);
                        let chunk: String = line_chars[pos..end].iter().collect();

                        // Check if cursor is in this chunk
                        let chunk_start = char_offset + pos;
                        let chunk_end = char_offset + end;
                        let cursor_col = if cursor_char_pos >= chunk_start && cursor_char_pos <= chunk_end {
                            cursor_display_line = Some(display_lines.len());
                            Some(cursor_char_pos - chunk_start)
                        } else {
                            None
                        };

                        display_lines.push(DisplayLine {
                            text: chunk,
                            cursor_col,
                        });

                        pos = end;
                    }
                }

                // Account for the newline character (except for the last line)
                char_offset += line_len;
                if line_idx < text_lines.len() - 1 {
                    char_offset += 1; // newline
                }
            }

            // If text is empty, add one empty line for cursor
            if display_lines.is_empty() {
                cursor_display_line = if is_selected { Some(0) } else { None };
                display_lines.push(DisplayLine {
                    text: String::new(),
                    cursor_col: if is_selected { Some(0) } else { None },
                });
            }

            // Calculate the correct scroll_offset based on actual wrap width
            // This fixes the mismatch between conservative scroll calculation and actual rendering
            let mut effective_scroll_offset = *scroll_offset;
            let total_display_lines = display_lines.len();

            if let Some(cursor_line) = cursor_display_line {
                // Ensure cursor is visible by adjusting scroll_offset
                if cursor_line < effective_scroll_offset {
                    effective_scroll_offset = cursor_line;
                } else if cursor_line >= effective_scroll_offset + *visible_lines {
                    effective_scroll_offset = cursor_line.saturating_sub(*visible_lines - 1);
                }
            }

            // Clamp scroll_offset to valid range
            let max_scroll = total_display_lines.saturating_sub(*visible_lines);
            effective_scroll_offset = effective_scroll_offset.min(max_scroll);

            // Value style - highlight background when selected
            let value_bg = if is_selected { theme.selection_bg() } else { theme.bg() };

            // Render visible display lines with scroll offset
            let mut rendered_lines: Vec<Line> = Vec::new();
            let start_line = effective_scroll_offset;

            for display_row in 0..*visible_lines {
                let line_idx = start_line + display_row;

                if line_idx < total_display_lines {
                    let dline = &display_lines[line_idx];

                    if let Some(cursor_col) = dline.cursor_col {
                        // This line has the cursor - render with '│' cursor like other text fields
                        let chars: Vec<char> = dline.text.chars().collect();
                        let cursor_pos = cursor_col.min(chars.len());
                        let before: String = chars[..cursor_pos].iter().collect();
                        let after: String = chars[cursor_pos..].iter().collect();

                        // Build line with cursor inserted
                        let line_with_cursor = format!("{}│{}", before, after);

                        // Pad to fill the value area width (account for the cursor character)
                        let content_len = before.chars().count() + 1 + after.chars().count();
                        let padding_needed = wrap_width.saturating_sub(content_len);
                        let padded_line = format!("{}{}", line_with_cursor, " ".repeat(padding_needed));

                        rendered_lines.push(Line::from(Span::styled(
                            padded_line,
                            Style::default().fg(theme.fg()).bg(value_bg),
                        )));
                    } else {
                        // Regular line without cursor
                        let content_len = dline.text.chars().count();
                        let padding_needed = wrap_width.saturating_sub(content_len);
                        let padded_line = format!("{}{}", dline.text, " ".repeat(padding_needed));
                        rendered_lines.push(Line::from(Span::styled(
                            padded_line,
                            Style::default().fg(theme.fg()).bg(value_bg),
                        )));
                    }
                } else {
                    // Empty line beyond content
                    let padding_str: String = " ".repeat(wrap_width);
                    rendered_lines.push(Line::from(Span::styled(
                        padding_str,
                        Style::default().bg(value_bg),
                    )));
                }
            }

            // Render the multiline content in the value area
            let content_area = Rect::new(value_start_x, area.y, value_width, *visible_lines as u16);
            f.render_widget(Paragraph::new(rendered_lines), content_area);
        }

        FieldKind::List { items, selected_index, scroll_offset, headers, column_widths, .. } => {
            // Use actual area height minus header row if present
            let header_rows = if headers.is_some() { 1 } else { 0 };
            let actual_visible = (area.height as usize).saturating_sub(header_rows);
            let highlight_range = state.highlight.as_ref().and_then(|h| {
                if h.field_id == field.id {
                    Some((h.start_line.min(h.end_line), h.start_line.max(h.end_line)))
                } else {
                    None
                }
            });
            render_list_field(f, items, *selected_index, *scroll_offset, actual_visible, headers, column_widths, area, is_selected, highlight_range, theme);
        }

        FieldKind::ScrollableContent { lines, scroll_offset, visible_height } => {
            let highlight_range = state.highlight.as_ref().and_then(|h| {
                if h.field_id == field.id {
                    Some((h.start_line.min(h.end_line), h.start_line.max(h.end_line)))
                } else {
                    None
                }
            });
            render_scrollable_content(f, lines, *scroll_offset, *visible_height, area, highlight_range, theme);
        }

        FieldKind::EditableList { items, selected_index, scroll_offset, visible_height } => {
            render_editable_list_field(f, state, field, items, *selected_index, *scroll_offset, *visible_height, area, is_selected, theme);
        }
    }
}

/// Render a labeled field (label: value)
/// When selected, only the value area is highlighted (not the label)
/// If shortcut is provided, that character in the label is underlined
#[allow(clippy::too_many_arguments)]
fn render_labeled_field_with_shortcut(
    f: &mut Frame,
    label: &str,
    value: &str,
    area: Rect,
    label_width: usize,
    is_selected: bool,
    shortcut: Option<char>,
    theme: &Theme,
) {
    let label_style = Style::default().fg(theme.fg_dim());
    let shortcut_style = Style::default().fg(theme.fg_dim()).add_modifier(Modifier::UNDERLINED);

    // Build label spans with optional underlined shortcut
    let mut label_spans: Vec<Span> = Vec::new();

    // Add padding before label
    let padding_width = label_width.saturating_sub(label.len() + 2);
    if padding_width > 0 {
        label_spans.push(Span::styled(" ".repeat(padding_width), label_style));
    }

    // Add label with optional shortcut underline
    if let Some(sc) = shortcut {
        let sc_lower = sc.to_ascii_lowercase();
        let mut found = false;
        for c in label.chars() {
            if !found && c.to_ascii_lowercase() == sc_lower {
                label_spans.push(Span::styled(c.to_string(), shortcut_style));
                found = true;
            } else {
                label_spans.push(Span::styled(c.to_string(), label_style));
            }
        }
    } else {
        label_spans.push(Span::styled(label.to_string(), label_style));
    }

    // Add colon and space
    if !label.is_empty() {
        label_spans.push(Span::styled(": ", label_style));
    }

    // Calculate label width for value positioning
    let label_chars: usize = label_spans.iter().map(|s| s.content.chars().count()).sum();

    // Calculate remaining width for value area (one char smaller on right)
    let value_area_width = (area.width as usize).saturating_sub(label_chars).saturating_sub(1);

    // Pad value to fill the value area
    let padded_value = format!("{:<width$}", value, width = value_area_width);

    let value_style = if is_selected {
        Style::default()
            .fg(theme.fg_accent())
            .bg(theme.selection_bg())
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.fg())
    };

    label_spans.push(Span::styled(padded_value, value_style));

    let line = Line::from(label_spans);
    f.render_widget(Paragraph::new(line), area);
}

/// Render a labeled field (label: value) - simple version without shortcut
fn render_labeled_field(
    f: &mut Frame,
    label: &str,
    value: &str,
    area: Rect,
    label_width: usize,
    is_selected: bool,
    theme: &Theme,
) {
    render_labeled_field_with_shortcut(f, label, value, area, label_width, is_selected, None, theme);
}

/// Render a list field with optional headers, aligned columns, and scrollbar
#[allow(clippy::too_many_arguments)]
fn render_list_field(
    f: &mut Frame,
    items: &[super::ListItem],
    selected_index: usize,
    scroll_offset: usize,
    visible_height: usize,
    headers: &Option<Vec<String>>,
    stored_column_widths: &Option<Vec<usize>>,
    area: Rect,
    _is_selected: bool,
    highlight_range: Option<(usize, usize)>,
    theme: &Theme,
) {
    let prefix_width = 2; // "* " or "  "

    // Reserve space for scrollbar on right
    let needs_scrollbar = items.len() > visible_height;
    let scrollbar_width = if needs_scrollbar { 1 } else { 0 };
    let content_width = area.width.saturating_sub(scrollbar_width as u16) as usize;

    // Use stored column widths if available, otherwise calculate from current items
    let col_widths: Vec<usize> = if let Some(widths) = stored_column_widths {
        widths.clone()
    } else {
        // Calculate column widths from headers and items
        let num_columns = headers.as_ref().map(|h| h.len()).unwrap_or_else(|| {
            items.first().map(|i| i.columns.len()).unwrap_or(1)
        });

        let mut widths: Vec<usize> = vec![0; num_columns];

        // Consider header widths
        if let Some(hdrs) = headers {
            for (i, h) in hdrs.iter().enumerate() {
                if i < widths.len() {
                    widths[i] = widths[i].max(h.len());
                }
            }
        }

        // Consider item widths
        for item in items {
            for (i, col) in item.columns.iter().enumerate() {
                if i < widths.len() {
                    widths[i] = widths[i].max(col.len());
                }
            }
        }
        widths
    };

    // Add spacing between columns
    let col_spacing = 2;

    let mut y = area.y;
    let header_y = y; // Track where header starts for scrollbar positioning

    // Render header row if present
    if let Some(hdrs) = headers {
        let mut header_text = String::new();
        header_text.push_str(&" ".repeat(prefix_width)); // Align with prefix

        for (i, h) in hdrs.iter().enumerate() {
            if i < col_widths.len() {
                header_text.push_str(&format!("{:<width$}", h, width = col_widths[i]));
                if i < hdrs.len() - 1 {
                    header_text.push_str(&" ".repeat(col_spacing));
                }
            }
        }

        let header_area = Rect::new(area.x, y, area.width.saturating_sub(scrollbar_width as u16), 1);
        let header_line = Line::from(Span::styled(
            truncate_str(&header_text, content_width),
            Style::default().fg(theme.fg_dim()).add_modifier(Modifier::BOLD),
        ));
        f.render_widget(Paragraph::new(header_line), header_area);
        y += 1;
    }

    // Calculate remaining height for items
    let items_height = if headers.is_some() {
        area.height.saturating_sub(1) as usize
    } else {
        area.height as usize
    };

    let actual_visible = visible_height.min(items_height);

    // Cap scroll_offset to ensure we don't show blank lines at bottom
    // Last item should be at the bottom when scrolled all the way down
    let max_scroll = items.len().saturating_sub(actual_visible);
    let effective_scroll = scroll_offset.min(max_scroll);

    let visible_items: Vec<_> = items
        .iter()
        .enumerate()
        .skip(effective_scroll)
        .take(actual_visible)
        .collect();

    for (i, (idx, item)) in visible_items.iter().enumerate() {
        let is_item_selected = *idx == selected_index;
        let row_y = y + i as u16;

        if row_y >= area.y + area.height {
            break;
        }

        // Row area extends to the scrollbar (highlight fills to scrollbar)
        let row_area = Rect::new(area.x, row_y, area.width.saturating_sub(scrollbar_width as u16), 1);

        // Build display text from columns with alignment
        let mut text = String::new();
        for (col_idx, col) in item.columns.iter().enumerate() {
            if col_idx < col_widths.len() {
                text.push_str(&format!("{:<width$}", col, width = col_widths[col_idx]));
                if col_idx < item.columns.len() - 1 {
                    text.push_str(&" ".repeat(col_spacing));
                }
            }
        }

        let prefix = if item.style.is_current { "* " } else { "  " };
        let full_text = format!("{}{}", prefix, text);

        // Pad to fill the entire row width (so highlight extends to scrollbar)
        let padded_text = format!("{:<width$}", full_text, width = content_width);

        let is_highlighted = highlight_range.is_some_and(|(start, end)| *idx >= start && *idx <= end);

        let style = if is_item_selected {
            Style::default()
                .fg(theme.fg_accent())
                .bg(theme.selection_bg())
                .add_modifier(Modifier::BOLD)
        } else if is_highlighted {
            Style::default()
                .fg(theme.fg())
                .bg(theme.selection_bg())
        } else if item.style.is_connected {
            Style::default().fg(theme.fg_success())
        } else if item.style.is_disabled {
            Style::default().fg(theme.fg_dim())
        } else {
            Style::default().fg(theme.fg())
        };

        let line = Line::from(Span::styled(
            truncate_str(&padded_text, content_width),
            style,
        ));

        f.render_widget(Paragraph::new(line), row_area);
    }

    // Render scrollbar on right edge if needed
    if needs_scrollbar {
        let scrollbar_x = area.x + area.width.saturating_sub(1);
        let list_start_y = if headers.is_some() { header_y + 1 } else { header_y };
        render_scrollbar_column(
            f, scrollbar_x, list_start_y, area.y + area.height,
            actual_visible, items.len(), effective_scroll, theme,
        );
    }
}

/// Draw a single-column vertical scrollbar: a `"█"` thumb over a `"│"` track, in
/// `theme.fg_dim()` — the shared visual style used by both list-style popups (`/worlds`,
/// see `render_list_field` above) and the field-list scrollbar for popups with more scalar
/// fields (Toggle/Select/Number/Text/...) than fit the screen (see `render_popup_content`).
///
/// `x`/`y`: top-left of the scrollbar column. `row_limit`: absolute row past which nothing
/// is drawn (caller's area boundary). `viewport`: rows visible at once. `total`: total
/// rows/items being scrolled over. `offset`: current scroll position, in the same units as
/// `total`/`viewport`, already clamped by the caller to `[0, total - viewport]`.
#[allow(clippy::too_many_arguments)]
fn render_scrollbar_column(
    f: &mut Frame,
    x: u16,
    y: u16,
    row_limit: u16,
    viewport: usize,
    total: usize,
    offset: usize,
    theme: &Theme,
) {
    if viewport == 0 || total == 0 {
        return;
    }
    let max_scroll = total.saturating_sub(viewport);
    let thumb_size = ((viewport as f64 / total as f64) * viewport as f64).max(1.0) as usize;
    let thumb_pos = if max_scroll == 0 {
        0
    } else {
        ((offset as f64 / max_scroll as f64) * viewport.saturating_sub(thumb_size) as f64) as usize
    };

    for i in 0..viewport {
        let row_y = y + i as u16;
        if row_y >= row_limit {
            break;
        }
        let ch = if i >= thumb_pos && i < thumb_pos + thumb_size { "█" } else { "│" };
        let line = Line::from(Span::styled(ch, Style::default().fg(theme.fg_dim())));
        f.render_widget(Paragraph::new(line), Rect::new(x, row_y, 1, 1));
    }
}

/// Render an EditableList field.
///
/// Shows `visible_height` rows at a time with a right-edge scrollbar when needed.
/// The currently selected row is always highlighted; when in edit mode for this field
/// the selected row additionally shows the shared `state.edit_buffer` with the text
/// cursor and horizontal scroll (identical to the Text field renderer).
#[allow(clippy::too_many_arguments)]
fn render_editable_list_field(
    f: &mut Frame,
    state: &PopupState,
    field: &super::Field,
    items: &[String],
    selected_index: usize,
    scroll_offset: usize,
    visible_height: usize,
    area: Rect,
    is_selected: bool,
    theme: &Theme,
) {
    let label_width = state.definition.layout.label_width;

    // Draw the label ("Patterns: ") to the left of the box
    // The box itself occupies the remainder of the row height
    let padding_width = label_width.saturating_sub(field.label.len() + 2);
    let label_total = padding_width + field.label.len() + 2; // padding + label + ": "

    // Label — rendered once in the top-left of the field area
    if !field.label.is_empty() {
        let label_style = Style::default().fg(theme.fg_dim());
        let label_str = format!(
            "{}{}: ",
            " ".repeat(padding_width),
            field.label
        );
        let label_area = Rect::new(area.x, area.y, label_total.min(area.width as usize) as u16, 1);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(label_str, label_style))),
            label_area,
        );
    }

    // The box starts after the label (first row) and uses full width on subsequent rows.
    // To keep it simple: shift the content area right by label_total on all rows.
    let box_x = area.x + label_total.min(area.width as usize) as u16;
    let box_width = area.width.saturating_sub(label_total as u16);
    if box_width == 0 || area.height == 0 {
        return;
    }

    let needs_scrollbar = items.len() > visible_height;
    let scrollbar_width = if needs_scrollbar { 1u16 } else { 0u16 };
    let content_width = box_width.saturating_sub(scrollbar_width) as usize;

    let actual_visible = visible_height.min(area.height as usize);
    let max_scroll = items.len().saturating_sub(actual_visible);
    let effective_scroll = scroll_offset.min(max_scroll);

    let is_editing_this = is_selected && state.editing;

    for i in 0..actual_visible {
        let item_idx = effective_scroll + i;
        let row_y = area.y + i as u16;
        if row_y >= area.y + area.height {
            break;
        }

        let row_area = Rect::new(box_x, row_y, box_width.saturating_sub(scrollbar_width), 1);
        let is_row_selected = item_idx == selected_index && is_selected;

        let display_text = if is_row_selected && is_editing_this {
            // Show the live edit buffer with cursor + horizontal scroll
            let chars: Vec<char> = state.edit_buffer.chars().collect();
            let cursor_pos = state.edit_cursor.min(chars.len());

            // Reserve 3 chars: cursor glyph + potential < > indicators
            let visible_width = content_width.saturating_sub(3);
            let scroll = if visible_width == 0 || cursor_pos <= visible_width {
                0
            } else {
                let margin = (visible_width / 4).max(1);
                cursor_pos.saturating_sub(visible_width - margin)
            };

            let has_left = scroll > 0;
            let visible_start = scroll;
            let visible_end = (scroll + visible_width).min(chars.len());
            let has_right = visible_end < chars.len();

            let mut result = String::new();
            if has_left { result.push('<'); }
            let visible_cursor = cursor_pos.saturating_sub(scroll);
            for (ii, c) in chars.iter().enumerate().skip(visible_start).take(visible_end - visible_start) {
                let idx = ii - visible_start;
                if idx == visible_cursor { result.push('│'); }
                result.push(*c);
            }
            if visible_cursor >= visible_end - visible_start { result.push('│'); }
            if has_right { result.push('>'); }
            result
        } else {
            // Idle display: show the stored item text, truncated to content_width
            let text = items.get(item_idx).map(|s| s.as_str()).unwrap_or("");
            truncate_str(text, content_width).to_string()
        };

        let padded = format!("{:<width$}", display_text, width = content_width);

        let style = if is_row_selected {
            Style::default()
                .fg(theme.fg_accent())
                .bg(theme.selection_bg())
                .add_modifier(Modifier::BOLD)
        } else if items.get(item_idx).map(|s| s.is_empty()).unwrap_or(true) {
            // Empty rows are dimmed when not selected
            Style::default().fg(theme.fg_dim())
        } else {
            Style::default().fg(theme.fg())
        };

        f.render_widget(
            Paragraph::new(Line::from(Span::styled(padded, style))),
            row_area,
        );
    }

    // Scrollbar
    if needs_scrollbar {
        let scrollbar_x = box_x + box_width.saturating_sub(1);
        let thumb_size = ((actual_visible as f64 / items.len() as f64) * actual_visible as f64).max(1.0) as usize;
        let thumb_pos = if max_scroll == 0 {
            0
        } else {
            ((effective_scroll as f64 / max_scroll as f64) * (actual_visible - thumb_size) as f64) as usize
        };

        for i in 0..actual_visible {
            let row_y = area.y + i as u16;
            if row_y >= area.y + area.height { break; }
            let scrollbar_area = Rect::new(scrollbar_x, row_y, 1, 1);
            let ch = if i >= thumb_pos && i < thumb_pos + thumb_size { "█" } else { "│" };
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(ch, Style::default().fg(theme.fg_dim())))),
                scrollbar_area,
            );
        }
    }
}

/// Render scrollable content (read-only text with vertical scrolling)
fn render_scrollable_content(
    f: &mut Frame,
    lines: &[String],
    scroll_offset: usize,
    visible_height: usize,
    area: Rect,
    highlight_range: Option<(usize, usize)>,
    theme: &Theme,
) {
    let actual_height = visible_height.min(area.height as usize);
    let total_lines = lines.len();

    // Render visible lines
    for (i, line) in lines.iter().skip(scroll_offset).take(actual_height).enumerate() {
        let row_y = area.y + i as u16;
        if row_y >= area.y + area.height {
            break;
        }

        let content_line_idx = scroll_offset + i;
        let is_highlighted = highlight_range.is_some_and(|(start, end)| content_line_idx >= start && content_line_idx <= end);

        let row_area = Rect::new(area.x, row_y, area.width.saturating_sub(1), 1);
        // Pad line to fill the row area (prevents background bleed-through)
        let display_text = truncate_str(line, area.width as usize - 2);
        let padded_text = format!("{:<width$}", display_text, width = row_area.width as usize);
        let style = if is_highlighted {
            Style::default().fg(theme.fg()).bg(theme.selection_bg())
        } else {
            Style::default().fg(theme.fg()).bg(theme.popup_bg())
        };
        let text_line = Line::from(Span::styled(padded_text, style));
        f.render_widget(Paragraph::new(text_line), row_area);
    }

    // Render scrollbar on right edge if needed
    if total_lines > actual_height {
        let scrollbar_x = area.x + area.width.saturating_sub(1);
        let thumb_size = (actual_height as f64 / total_lines as f64 * actual_height as f64).max(1.0) as usize;
        let thumb_pos = if total_lines <= actual_height {
            0
        } else {
            (scroll_offset as f64 / (total_lines - actual_height) as f64 * (actual_height - thumb_size) as f64) as usize
        };

        for i in 0..actual_height {
            let row_y = area.y + i as u16;
            if row_y >= area.y + area.height {
                break;
            }

            let scrollbar_area = Rect::new(scrollbar_x, row_y, 1, 1);
            let char = if i >= thumb_pos && i < thumb_pos + thumb_size {
                "█"
            } else {
                "│"
            };
            let scrollbar_line = Line::from(Span::styled(
                char,
                Style::default().fg(theme.fg_dim()).bg(theme.popup_bg()),
            ));
            f.render_widget(Paragraph::new(scrollbar_line), scrollbar_area);
        }
    }
}

/// Render button row
fn build_button_spans(button: &super::Button, state: &PopupState, theme: &Theme) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let is_selected = matches!(&state.selected, ElementSelection::Button(id) if *id == button.id);

    if let Some(shortcut) = button.shortcut {
        let shortcut_lower = shortcut.to_ascii_lowercase();
        let shortcut_upper = shortcut.to_ascii_uppercase();

        if let Some(pos) = button.label.find([shortcut_lower, shortcut_upper]) {
            let (before, rest) = button.label.split_at(pos);
            let (shortcut_char, after) = rest.split_at(1);

            let base_style = get_button_style(button.style, is_selected, theme);
            let shortcut_style = base_style.add_modifier(Modifier::UNDERLINED);

            spans.push(Span::styled(format!("[{}", before), base_style));
            spans.push(Span::styled(shortcut_char.to_string(), shortcut_style));
            spans.push(Span::styled(format!("{}]", after), base_style));
        } else {
            let style = get_button_style(button.style, is_selected, theme);
            spans.push(Span::styled(format!("[{}]", button.label), style));
        }
    } else {
        let style = get_button_style(button.style, is_selected, theme);
        spans.push(Span::styled(format!("[{}]", button.label), style));
    }

    spans
}

fn render_buttons(f: &mut Frame, state: &mut PopupState, area: Rect, theme: &Theme) {
    let button_spacing = "  ";
    let bg_style = Style::default().bg(theme.popup_bg());

    // Collect button widths for hit area tracking
    let button_widths: Vec<(super::ButtonId, usize, bool)> = state.definition.buttons.iter()
        .filter(|b| b.enabled)
        .map(|b| {
            let width = b.label.len() + 2; // "[label]"
            (b.id, width, b.left_align)
        })
        .collect();

    // Split buttons into left-aligned and right-aligned groups
    let has_left_buttons = state.definition.buttons.iter().any(|b| b.enabled && b.left_align);

    if has_left_buttons {
        // Split layout: left buttons on left, right buttons on right
        let mut left_spans: Vec<Span<'static>> = Vec::new();
        let mut right_spans: Vec<Span<'static>> = Vec::new();

        for button in &state.definition.buttons {
            if !button.enabled { continue; }
            let target = if button.left_align { &mut left_spans } else { &mut right_spans };
            if !target.is_empty() {
                target.push(Span::styled(button_spacing.to_string(), bg_style));
            }
            target.extend(build_button_spans(button, state, theme));
        }

        let left_width: usize = left_spans.iter().map(|s| s.content.len()).sum();
        let right_width: usize = right_spans.iter().map(|s| s.content.len()).sum();
        let gap = (area.width as usize).saturating_sub(left_width + right_width + 2); // 1 margin each side

        // Track button positions for hit areas
        let mut x_pos = area.x + 1; // left margin
        for &(btn_id, btn_width, is_left) in &button_widths {
            if is_left {
                let prev_x = x_pos;
                if prev_x > area.x + 1 { x_pos += 2; } // spacing
                state.hit_areas.push((
                    Rect::new(x_pos, area.y, btn_width as u16, 1),
                    ElementSelection::Button(btn_id),
                ));
                x_pos += btn_width as u16;
            }
        }
        // Right-aligned buttons start after gap
        x_pos = area.x + 1 + left_width as u16 + gap as u16;
        for &(btn_id, btn_width, is_left) in &button_widths {
            if !is_left {
                let prev_x = x_pos;
                if prev_x > area.x + 1 + left_width as u16 + gap as u16 { x_pos += 2; }
                state.hit_areas.push((
                    Rect::new(x_pos, area.y, btn_width as u16, 1),
                    ElementSelection::Button(btn_id),
                ));
                x_pos += btn_width as u16;
            }
        }

        let mut positioned_spans = Vec::new();
        positioned_spans.push(Span::styled(" ".to_string(), bg_style)); // left margin
        positioned_spans.extend(left_spans);
        positioned_spans.push(Span::styled(" ".repeat(gap), bg_style));
        positioned_spans.extend(right_spans);

        // Trailing padding to fill the row
        let used_width: usize = positioned_spans.iter().map(|s| s.content.len()).sum();
        let trailing = (area.width as usize).saturating_sub(used_width);
        if trailing > 0 {
            positioned_spans.push(Span::styled(" ".repeat(trailing), bg_style));
        }

        let line = Line::from(positioned_spans);
        f.render_widget(Paragraph::new(line).style(bg_style), area);
    } else {
        // Original layout: all buttons together (right-aligned or centered)
        let mut spans: Vec<Span<'static>> = Vec::new();

        for button in &state.definition.buttons {
            if !button.enabled { continue; }
            if !spans.is_empty() {
                spans.push(Span::styled(button_spacing.to_string(), bg_style));
            }
            spans.extend(build_button_spans(button, state, theme));
        }

        let total_width: usize = spans.iter().map(|s| s.content.len()).sum();
        let padding = if state.definition.layout.buttons_right_align {
            (area.width as usize).saturating_sub(total_width).saturating_sub(1)
        } else {
            (area.width as usize).saturating_sub(total_width) / 2
        };

        // Track button positions for hit areas
        let mut x_pos = area.x + padding as u16;
        for &(btn_id, btn_width, _) in &button_widths {
            state.hit_areas.push((
                Rect::new(x_pos, area.y, btn_width as u16, 1),
                ElementSelection::Button(btn_id),
            ));
            x_pos += btn_width as u16 + 2; // button width + spacing
        }

        let mut positioned_spans = vec![Span::styled(" ".repeat(padding), bg_style)];
        positioned_spans.extend(spans);

        let used_width: usize = positioned_spans.iter().map(|s| s.content.len()).sum();
        let trailing = (area.width as usize).saturating_sub(used_width);
        if trailing > 0 {
            positioned_spans.push(Span::styled(" ".repeat(trailing), bg_style));
        }

        let line = Line::from(positioned_spans);
        f.render_widget(Paragraph::new(line).style(bg_style), area);
    }
}

/// Get button style based on type and selection state
fn get_button_style(button_style: ButtonStyle, is_selected: bool, theme: &Theme) -> Style {
    let base = match button_style {
        ButtonStyle::Primary => Style::default().fg(theme.fg_accent()).bg(theme.popup_bg()),
        ButtonStyle::Danger => Style::default().fg(theme.fg_error()).bg(theme.popup_bg()),
        ButtonStyle::Secondary => Style::default().fg(theme.fg()).bg(theme.popup_bg()),
    };

    if is_selected {
        base.add_modifier(Modifier::BOLD | Modifier::REVERSED)
    } else {
        base
    }
}

/// Truncate a string to fit within max_width
fn truncate_str(s: &str, max_width: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_width {
        s.to_string()
    } else if max_width > 3 {
        let truncated: String = chars[..max_width - 3].iter().collect();
        format!("{}...", truncated)
    } else {
        chars[..max_width].iter().collect()
    }
}

// ============================================================================
// Direct crossterm popup content renderer (bypasses ratatui for fast scroll)
// ============================================================================

/// Convert a ratatui Color to a crossterm Color
fn to_crossterm_color(c: ratatui::style::Color) -> crossterm::style::Color {
    match c {
        ratatui::style::Color::Reset => crossterm::style::Color::Reset,
        ratatui::style::Color::Black => crossterm::style::Color::Black,
        ratatui::style::Color::Red => crossterm::style::Color::DarkRed,
        ratatui::style::Color::Green => crossterm::style::Color::DarkGreen,
        ratatui::style::Color::Yellow => crossterm::style::Color::DarkYellow,
        ratatui::style::Color::Blue => crossterm::style::Color::DarkBlue,
        ratatui::style::Color::Magenta => crossterm::style::Color::DarkMagenta,
        ratatui::style::Color::Cyan => crossterm::style::Color::DarkCyan,
        ratatui::style::Color::Gray => crossterm::style::Color::Grey,
        ratatui::style::Color::DarkGray => crossterm::style::Color::DarkGrey,
        ratatui::style::Color::LightRed => crossterm::style::Color::Red,
        ratatui::style::Color::LightGreen => crossterm::style::Color::Green,
        ratatui::style::Color::LightYellow => crossterm::style::Color::Yellow,
        ratatui::style::Color::LightBlue => crossterm::style::Color::Blue,
        ratatui::style::Color::LightMagenta => crossterm::style::Color::Magenta,
        ratatui::style::Color::LightCyan => crossterm::style::Color::Cyan,
        ratatui::style::Color::White => crossterm::style::Color::White,
        ratatui::style::Color::Rgb(r, g, b) => crossterm::style::Color::Rgb { r, g, b },
        ratatui::style::Color::Indexed(i) => crossterm::style::Color::AnsiValue(i),
    }
}

/// Render just the scrollable/list content of a popup directly to stdout using crossterm.
/// This is much faster than a full ratatui terminal.draw() cycle because it skips
/// output area, separator bar, and input area rendering.
pub fn render_popup_content_direct(state: &PopupState, theme: &Theme) {
    use std::io::Write;
    use crossterm::{cursor::MoveTo, style::{SetForegroundColor, SetBackgroundColor, SetAttribute, Print, Attribute}, QueueableCommand};

    if !state.visible {
        return;
    }

    let mut stdout = std::io::stdout();

    // Find content areas to re-render (populated during last full render)
    if state.content_areas.is_empty() {
        return;
    }

    let popup_bg = to_crossterm_color(theme.popup_bg());

    for field in &state.definition.fields {
        match &field.kind {
            FieldKind::List { items, selected_index, scroll_offset, visible_height, headers, column_widths, .. } => {
                // Find the content area for this field
                let ca = state.content_areas.iter().find(|ca| ca.field_id == field.id);
                let area = match ca {
                    Some(ca) => ca.area,
                    None => continue,
                };

                let needs_scrollbar = items.len() > *visible_height;
                let scrollbar_width: u16 = if needs_scrollbar { 1 } else { 0 };
                let content_width = area.width.saturating_sub(scrollbar_width) as usize;

                let col_widths: Vec<usize> = if let Some(widths) = column_widths {
                    widths.clone()
                } else {
                    let num_columns = headers.as_ref().map(|h| h.len()).unwrap_or_else(|| {
                        items.first().map(|i| i.columns.len()).unwrap_or(1)
                    });
                    let mut widths = vec![0; num_columns];
                    if let Some(hdrs) = headers {
                        for (i, h) in hdrs.iter().enumerate() {
                            if i < widths.len() { widths[i] = widths[i].max(h.len()); }
                        }
                    }
                    for item in items.iter() {
                        for (i, col) in item.columns.iter().enumerate() {
                            if i < widths.len() { widths[i] = widths[i].max(col.len()); }
                        }
                    }
                    widths
                };

                let col_spacing = 2;
                // content_area already excludes header rows, so use area.y directly
                let items_start_y = area.y;
                let actual_visible = (*visible_height).min(area.height as usize);

                let max_scroll = items.len().saturating_sub(actual_visible);
                let effective_scroll = (*scroll_offset).min(max_scroll);

                // Render visible items
                for i in 0..actual_visible {
                    let row_y = items_start_y + i as u16;
                    let item_idx = effective_scroll + i;

                    let _ = stdout.queue(MoveTo(area.x, row_y));

                    if item_idx < items.len() {
                        let item = &items[item_idx];
                        let is_item_selected = item_idx == *selected_index;

                        // Choose colors
                        let (fg, bg, bold) = if is_item_selected {
                            (to_crossterm_color(theme.fg_accent()), to_crossterm_color(theme.selection_bg()), true)
                        } else if item.style.is_connected {
                            (to_crossterm_color(theme.fg_success()), popup_bg, false)
                        } else if item.style.is_disabled {
                            (to_crossterm_color(theme.fg_dim()), popup_bg, false)
                        } else {
                            (to_crossterm_color(theme.fg()), popup_bg, false)
                        };

                        // Build text
                        let mut text = String::new();
                        for (col_idx, col) in item.columns.iter().enumerate() {
                            if col_idx < col_widths.len() {
                                text.push_str(&format!("{:<width$}", col, width = col_widths[col_idx]));
                                if col_idx < item.columns.len() - 1 {
                                    text.push_str(&" ".repeat(col_spacing));
                                }
                            }
                        }

                        let prefix = if item.style.is_current { "* " } else { "  " };
                        let full_text = format!("{}{}", prefix, text);
                        let padded = format!("{:<width$}", truncate_str(&full_text, content_width), width = content_width);

                        let _ = stdout.queue(SetForegroundColor(fg));
                        let _ = stdout.queue(SetBackgroundColor(bg));
                        if bold {
                            let _ = stdout.queue(SetAttribute(Attribute::Bold));
                        }
                        let _ = stdout.queue(Print(&padded));
                        if bold {
                            let _ = stdout.queue(SetAttribute(Attribute::NoBold));
                        }
                    } else {
                        // Empty row (shouldn't happen but fill with background)
                        let _ = stdout.queue(SetForegroundColor(to_crossterm_color(theme.fg())));
                        let _ = stdout.queue(SetBackgroundColor(popup_bg));
                        let _ = stdout.queue(Print(" ".repeat(content_width)));
                    }

                    // Scrollbar
                    if needs_scrollbar {
                        let thumb_size = ((actual_visible as f64 / items.len() as f64) * actual_visible as f64).max(1.0) as usize;
                        let thumb_pos = if max_scroll == 0 { 0 } else {
                            ((effective_scroll as f64 / max_scroll as f64) * (actual_visible - thumb_size) as f64) as usize
                        };
                        let ch = if i >= thumb_pos && i < thumb_pos + thumb_size { "█" } else { "│" };
                        let _ = stdout.queue(SetForegroundColor(to_crossterm_color(theme.fg_dim())));
                        let _ = stdout.queue(SetBackgroundColor(popup_bg));
                        let _ = stdout.queue(Print(ch));
                    }
                }

                let _ = stdout.queue(SetAttribute(Attribute::Reset));
            }

            FieldKind::ScrollableContent { lines, scroll_offset, visible_height, .. } => {
                let ca = state.content_areas.iter().find(|ca| ca.field_id == field.id);
                let area = match ca {
                    Some(ca) => ca.area,
                    None => continue,
                };

                let actual_height = (*visible_height).min(area.height as usize);
                let total_lines = lines.len();
                let scrollbar_width: u16 = if total_lines > actual_height { 1 } else { 0 };

                for i in 0..actual_height {
                    let row_y = area.y + i as u16;
                    let line_idx = scroll_offset + i;

                    let _ = stdout.queue(MoveTo(area.x, row_y));
                    let _ = stdout.queue(SetForegroundColor(to_crossterm_color(theme.fg())));
                    let _ = stdout.queue(SetBackgroundColor(popup_bg));

                    let row_width = area.width.saturating_sub(scrollbar_width) as usize;
                    if line_idx < lines.len() {
                        let display = truncate_str(&lines[line_idx], row_width);
                        let padded = format!("{:<width$}", display, width = row_width);
                        let _ = stdout.queue(Print(&padded));
                    } else {
                        let _ = stdout.queue(Print(" ".repeat(row_width)));
                    }

                    // Scrollbar
                    if total_lines > actual_height {
                        let thumb_size = ((actual_height as f64 / total_lines as f64) * actual_height as f64).max(1.0) as usize;
                        let max_scroll = total_lines.saturating_sub(actual_height);
                        let thumb_pos = if max_scroll == 0 { 0 } else {
                            ((*scroll_offset as f64 / max_scroll as f64) * (actual_height - thumb_size) as f64) as usize
                        };
                        let ch = if i >= thumb_pos && i < thumb_pos + thumb_size { "█" } else { "│" };
                        let _ = stdout.queue(SetForegroundColor(to_crossterm_color(theme.fg_dim())));
                        let _ = stdout.queue(SetBackgroundColor(popup_bg));
                        let _ = stdout.queue(Print(ch));
                    }
                }

                let _ = stdout.queue(SetAttribute(Attribute::Reset));
            }

            _ => {}
        }
    }

    let _ = stdout.flush();
}

// ============================================================================
// Specialized popup renderers
// ============================================================================

/// Render the filter popup (positioned in upper right)
pub fn render_filter_popup_new(
    f: &mut Frame,
    filter_text: &str,
    cursor: usize,
    theme: &Theme,
) {
    let area = f.size();

    // Small popup in upper right corner
    let popup_width = 40u16.min(area.width);
    let popup_height = 3u16;

    let x = area.width.saturating_sub(popup_width);
    let y = 0;

    let popup_area = Rect::new(x, y, popup_width, popup_height);

    // Clear the background
    f.render_widget(Clear, popup_area);

    // Show filter text with cursor
    let mut display_text = filter_text.to_string();
    let cursor_pos = cursor.min(display_text.chars().count());
    let (before, after): (String, String) = {
        let chars: Vec<char> = display_text.chars().collect();
        let before: String = chars[..cursor_pos].iter().collect();
        let after: String = chars[cursor_pos..].iter().collect();
        (before, after)
    };
    display_text = format!("{}│{}", before, after);

    let lines = vec![Line::from(vec![
        Span::styled("Filter: ", Style::default().fg(theme.fg_accent())),
        Span::styled(display_text, Style::default().fg(theme.fg())),
    ])];

    let popup_block = Block::default()
        .title(" Find [Esc to close] ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.popup_border()))
        .style(Style::default().bg(theme.popup_bg()));

    let popup_text = Paragraph::new(lines).block(popup_block);

    f.render_widget(popup_text, popup_area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::popup::{Button, ButtonId, Field, FieldId, PopupDefinition, PopupId};

    #[test]
    fn test_truncate_str() {
        assert_eq!(truncate_str("hello", 10), "hello");
        assert_eq!(truncate_str("hello world", 8), "hello...");
        assert_eq!(truncate_str("hi", 2), "hi");
        assert_eq!(truncate_str("abc", 3), "abc");
    }

    #[test]
    fn test_calculate_content_height() {
        let def = PopupDefinition::new(PopupId("test"), "Test")
            .with_field(Field::new(FieldId(1), "Name", FieldKind::text("")))
            .with_field(Field::new(FieldId(2), "Email", FieldKind::text("")))
            .with_button(Button::new(ButtonId(1), "OK"));

        let state = crate::popup::PopupState::new(def);

        let height = calculate_content_height(&state);
        // 2 fields + 2 for button row (blank + buttons)
        assert_eq!(height, 4);
    }

    /// Build a popup definition with `n` single-row toggle fields (ids 1..=n) and a Save
    /// button — the shape /setup and /world -e are in (many scalar fields, no List).
    fn many_field_def(n: u32) -> PopupDefinition {
        let mut def = PopupDefinition::new(PopupId("test"), "Test");
        for i in 1..=n {
            def = def.with_field(Field::new(FieldId(i), &format!("Field {i}"), FieldKind::toggle(false)));
        }
        def.with_button(Button::new(ButtonId(1), "Save"))
    }

    #[test]
    fn test_compute_field_layout_no_scroll_when_it_fits() {
        let def = many_field_def(3);
        let selected = ElementSelection::Field(FieldId(1));
        // area_height=5, has_buttons -> button_space=2 -> visible_rows=3; 3 fields fit exactly.
        let (rows, total, needs_scroll, offset) = compute_field_layout(&def, &selected, 0, 5, true);
        assert_eq!(rows.len(), 3);
        assert_eq!(total, 3);
        assert!(!needs_scroll, "3 rows in a 3-row viewport should NOT need scrolling (boundary case)");
        assert_eq!(offset, 0);
    }

    #[test]
    fn test_compute_field_layout_scrolls_to_reveal_selected_field() {
        let def = many_field_def(10);
        // area_height=5, has_buttons -> visible_rows=3; 10 fields >> 3, must scroll.
        let selected_last = ElementSelection::Field(FieldId(10));
        let (rows, total, needs_scroll, offset) = compute_field_layout(&def, &selected_last, 0, 5, true);
        assert_eq!(rows.len(), 10);
        assert_eq!(total, 10);
        assert!(needs_scroll);
        // Selecting the last field (row_start=9, height=1) must bring it fully into a
        // 3-row viewport: offset = 9 + 1 - 3 = 7.
        assert_eq!(offset, 7, "scrolling down should reveal the last field, not just clamp to 0");

        // Now selecting the first field from that scrolled-down position must scroll back up.
        let selected_first = ElementSelection::Field(FieldId(1));
        let (_, _, _, offset2) = compute_field_layout(&def, &selected_first, offset, 5, true);
        assert_eq!(offset2, 0, "scrolling up should reveal the first field, not stay scrolled down");
    }

    #[test]
    fn test_compute_field_layout_never_scrolls_past_end() {
        let def = many_field_def(10);
        let selected = ElementSelection::Field(FieldId(10));
        // Start with an already-huge stale offset (e.g. left over from a taller terminal) —
        // must clamp down to the real max, not stay pinned to something past the content.
        let (_, total, needs_scroll, offset) = compute_field_layout(&def, &selected, 9999, 5, true);
        assert!(needs_scroll);
        let visible_rows = 5usize.saturating_sub(2); // button_space=2
        assert_eq!(offset, total.saturating_sub(visible_rows));
    }

    #[test]
    fn test_compute_field_layout_no_buttons_uses_full_height() {
        let def_no_buttons = {
            let mut def = PopupDefinition::new(PopupId("test"), "Test");
            for i in 1..=5u32 {
                def = def.with_field(Field::new(FieldId(i), &format!("Field {i}"), FieldKind::toggle(false)));
            }
            def
        };
        let selected = ElementSelection::Field(FieldId(1));
        // No buttons -> button_space=0 -> visible_rows == area_height == 5; 5 fields fit exactly.
        let (_, total, needs_scroll, _) = compute_field_layout(&def_no_buttons, &selected, 0, 5, false);
        assert_eq!(total, 5);
        assert!(!needs_scroll);
    }

    #[test]
    fn test_compute_field_layout_no_selection_keeps_offset_clamped() {
        let def = many_field_def(10);
        // Nothing selected (e.g. only buttons focusable in some popup) — offset should still
        // be clamped to a valid range rather than left at an out-of-bounds stale value.
        let (_, total, needs_scroll, offset) = compute_field_layout(&def, &ElementSelection::None, 9999, 5, true);
        let visible_rows = 5usize.saturating_sub(2);
        assert!(needs_scroll);
        assert_eq!(offset, total.saturating_sub(visible_rows));
    }
}
