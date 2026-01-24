//! Console renderer for popups using ratatui
//!
//! Renders PopupState to the terminal using ratatui Frame.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use super::{
    ButtonStyle, ElementSelection, FieldKind, PopupLayout, PopupState,
};
use crate::encoding::Theme;

/// Render a popup to the console
pub fn render_popup(f: &mut Frame, state: &PopupState, theme: Theme) {
    if !state.visible {
        return;
    }

    let area = f.area();
    let layout = &state.definition.layout;

    // Calculate popup dimensions
    let (popup_area, inner_area) = calculate_popup_area(area, layout, state);

    // Clear background
    f.render_widget(Clear, popup_area);

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
    let popup_height = (content_height as u16 + 4) // +2 for borders, +2 for padding
        .min(area.height.saturating_sub(2))
        .max(5);

    // Calculate position
    let x = if layout.center_horizontal {
        area.width.saturating_sub(popup_width) / 2
    } else {
        // Position in upper right for non-centered popups (like filter)
        area.width.saturating_sub(popup_width)
    };

    let y = if layout.center_vertical {
        area.height.saturating_sub(popup_height) / 2
    } else {
        0
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
            _ => layout.label_width + 20,
        };

        max_width = max_width.max(field_width);
    }

    max_width
}

/// Calculate required content height
fn calculate_content_height(state: &PopupState) -> usize {
    let mut height = 0;

    for field in &state.definition.fields {
        if !field.visible {
            continue;
        }

        height += match &field.kind {
            FieldKind::Separator => 1,
            FieldKind::Label { text } => text.lines().count().max(1),
            FieldKind::MultilineText { line_count, .. } => *line_count,
            FieldKind::List { visible_height, .. } => *visible_height,
            _ => 1,
        };
    }

    // Add button row if there are buttons
    if !state.definition.buttons.is_empty() {
        height += 2; // One blank line + button row
    }

    height
}

/// Render popup content (fields and buttons)
fn render_popup_content(f: &mut Frame, state: &PopupState, area: Rect, theme: Theme) {
    let mut y = area.y;
    let available_width = area.width as usize;

    // Render fields
    for field in &state.definition.fields {
        if !field.visible {
            continue;
        }

        let is_selected = matches!(&state.selected, ElementSelection::Field(id) if *id == field.id);

        let field_height = match &field.kind {
            FieldKind::Label { text } => text.lines().count().max(1) as u16,
            FieldKind::MultilineText { line_count, .. } => *line_count as u16,
            FieldKind::List { visible_height, .. } => *visible_height as u16,
            _ => 1,
        };

        let field_area = Rect::new(area.x, y, area.width, field_height);
        render_field(f, state, field, field_area, is_selected, theme);

        y += field_height;
    }

    // Render buttons if present
    if !state.definition.buttons.is_empty() {
        y += 1; // Blank line before buttons

        if y < area.y + area.height {
            let button_area = Rect::new(area.x, y, area.width, 1);
            render_buttons(f, state, button_area, theme);
        }
    }

    // Render error message if present
    if let Some(error) = &state.error {
        let error_y = area.y + area.height.saturating_sub(1);
        let error_area = Rect::new(area.x, error_y, area.width, 1);
        let error_line = Line::from(Span::styled(
            truncate_str(error, available_width),
            Style::default().fg(Color::Red),
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
    theme: Theme,
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
            let display_value = if state.editing && is_selected {
                // Show edit buffer with cursor
                let mut buf = state.edit_buffer.clone();
                if *masked {
                    buf = "*".repeat(buf.len());
                }
                // Insert cursor
                let cursor_pos = state.edit_cursor.min(buf.chars().count());
                let (before, after): (String, String) = {
                    let chars: Vec<char> = buf.chars().collect();
                    let before: String = chars[..cursor_pos].iter().collect();
                    let after: String = chars[cursor_pos..].iter().collect();
                    (before, after)
                };
                format!("{}│{}", before, after)
            } else if value.is_empty() {
                placeholder.clone().unwrap_or_default()
            } else if *masked {
                "*".repeat(value.len())
            } else {
                value.clone()
            };

            render_labeled_field(f, &field.label, &display_value, area, label_width, is_selected, theme);
        }

        FieldKind::Toggle { value } => {
            let display_value = if *value { "[✓]" } else { "[ ]" };
            render_labeled_field(f, &field.label, display_value, area, label_width, is_selected, theme);
        }

        FieldKind::Select { options, selected_index } => {
            let display_value = options
                .get(*selected_index)
                .map(|o| format!("< {} >", o.label))
                .unwrap_or_else(|| "< - >".to_string());
            render_labeled_field(f, &field.label, &display_value, area, label_width, is_selected, theme);
        }

        FieldKind::Number { value, min, max } => {
            let display_value = format!(
                "< {} >{}",
                value,
                match (min, max) {
                    (Some(lo), Some(hi)) => format!(" ({}-{})", lo, hi),
                    _ => String::new(),
                }
            );
            render_labeled_field(f, &field.label, &display_value, area, label_width, is_selected, theme);
        }

        FieldKind::MultilineText { value, line_count } => {
            // TODO: Implement multiline text rendering with scrolling
            let lines: Vec<Line> = value
                .lines()
                .take(*line_count)
                .map(|l| Line::from(Span::styled(l.to_string(), Style::default().fg(theme.fg()))))
                .collect();
            f.render_widget(Paragraph::new(lines), area);
        }

        FieldKind::List { items, selected_index, scroll_offset, visible_height } => {
            render_list_field(f, items, *selected_index, *scroll_offset, *visible_height, area, is_selected, theme);
        }

        FieldKind::ScrollableContent { lines, scroll_offset, visible_height } => {
            render_scrollable_content(f, lines, *scroll_offset, *visible_height, area, theme);
        }
    }
}

/// Render a labeled field (label: value)
fn render_labeled_field(
    f: &mut Frame,
    label: &str,
    value: &str,
    area: Rect,
    label_width: usize,
    is_selected: bool,
    theme: Theme,
) {
    let padded_label = if label.is_empty() {
        String::new()
    } else {
        format!("{:>width$}: ", label, width = label_width.saturating_sub(2))
    };

    let label_style = Style::default().fg(theme.fg_dim());
    let value_style = if is_selected {
        Style::default()
            .fg(theme.fg_accent())
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.fg())
    };

    let line = Line::from(vec![
        Span::styled(padded_label, label_style),
        Span::styled(value.to_string(), value_style),
    ]);

    let bg_style = if is_selected {
        Style::default().bg(theme.selection_bg())
    } else {
        Style::default()
    };

    f.render_widget(Paragraph::new(line).style(bg_style), area);
}

/// Render a list field
fn render_list_field(
    f: &mut Frame,
    items: &[super::ListItem],
    selected_index: usize,
    scroll_offset: usize,
    visible_height: usize,
    area: Rect,
    _is_selected: bool,
    theme: Theme,
) {
    let visible_items = items
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(visible_height.min(area.height as usize));

    for (i, (idx, item)) in visible_items.enumerate() {
        let is_item_selected = idx == selected_index;
        let row_y = area.y + i as u16;

        if row_y >= area.y + area.height {
            break;
        }

        let row_area = Rect::new(area.x, row_y, area.width, 1);

        // Build display text from columns
        let text: String = item.columns.join(" │ ");

        let style = if is_item_selected {
            Style::default()
                .fg(theme.fg_accent())
                .bg(theme.selection_bg())
                .add_modifier(Modifier::BOLD)
        } else if item.style.is_connected {
            Style::default().fg(Color::Green)
        } else if item.style.is_disabled {
            Style::default().fg(theme.fg_dim())
        } else {
            Style::default().fg(theme.fg())
        };

        let prefix = if item.style.is_current { "* " } else { "  " };
        let line = Line::from(Span::styled(
            format!("{}{}", prefix, truncate_str(&text, area.width as usize - 2)),
            style,
        ));

        f.render_widget(Paragraph::new(line), row_area);
    }
}

/// Render scrollable content (read-only text with vertical scrolling)
fn render_scrollable_content(
    f: &mut Frame,
    lines: &[String],
    scroll_offset: usize,
    visible_height: usize,
    area: Rect,
    theme: Theme,
) {
    let actual_height = visible_height.min(area.height as usize);
    let total_lines = lines.len();

    // Render visible lines
    for (i, line) in lines.iter().skip(scroll_offset).take(actual_height).enumerate() {
        let row_y = area.y + i as u16;
        if row_y >= area.y + area.height {
            break;
        }

        let row_area = Rect::new(area.x, row_y, area.width.saturating_sub(1), 1);
        let text_line = Line::from(Span::styled(
            truncate_str(line, area.width as usize - 2),
            Style::default().fg(theme.fg()),
        ));
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
            let scrollbar_line = Line::from(Span::styled(char, Style::default().fg(theme.fg_dim())));
            f.render_widget(Paragraph::new(scrollbar_line), scrollbar_area);
        }
    }
}

/// Render button row
fn render_buttons(f: &mut Frame, state: &PopupState, area: Rect, theme: Theme) {
    let mut spans = Vec::new();
    let button_spacing = "  ";

    for button in &state.definition.buttons {
        if !button.enabled {
            continue;
        }

        let is_selected = matches!(&state.selected, ElementSelection::Button(id) if *id == button.id);

        // Build button label with shortcut highlight
        if let Some(shortcut) = button.shortcut {
            // Find and highlight shortcut character
            let shortcut_lower = shortcut.to_ascii_lowercase();
            let shortcut_upper = shortcut.to_ascii_uppercase();

            if let Some(pos) = button.label.find(|c: char| c == shortcut_lower || c == shortcut_upper) {
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

        spans.push(Span::raw(button_spacing.to_string()));
    }

    // Remove trailing spacing
    if !spans.is_empty() {
        spans.pop();
    }

    // Center buttons
    let total_width: usize = spans.iter().map(|s| s.content.len()).sum();
    let padding = (area.width as usize).saturating_sub(total_width) / 2;

    let mut centered_spans = vec![Span::raw(" ".repeat(padding))];
    centered_spans.extend(spans);

    let line = Line::from(centered_spans);
    f.render_widget(Paragraph::new(line), area);
}

/// Get button style based on type and selection state
fn get_button_style(button_style: ButtonStyle, is_selected: bool, theme: Theme) -> Style {
    let base = match button_style {
        ButtonStyle::Primary => Style::default().fg(theme.fg_accent()),
        ButtonStyle::Danger => Style::default().fg(Color::Red),
        ButtonStyle::Secondary => Style::default().fg(theme.fg()),
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
// Specialized popup renderers
// ============================================================================

/// Render the filter popup (positioned in upper right)
pub fn render_filter_popup_new(
    f: &mut Frame,
    filter_text: &str,
    cursor: usize,
    theme: Theme,
) {
    let area = f.area();

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
}
