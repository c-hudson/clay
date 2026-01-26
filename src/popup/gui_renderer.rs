//! GUI renderer for popups using egui
//!
//! Renders PopupState to an egui context.

use egui::{Color32, RichText, Ui};

use super::{ButtonStyle, ElementSelection, FieldId, FieldKind, PopupState};

/// Theme colors for GUI popup rendering
#[derive(Clone, Copy)]
pub struct GuiPopupTheme {
    pub bg_elevated: Color32,
    pub bg_surface: Color32,
    pub bg_deep: Color32,
    pub bg_hover: Color32,
    pub fg_primary: Color32,
    pub fg_secondary: Color32,
    pub fg_dim: Color32,
    pub accent: Color32,
    pub accent_dim: Color32,
    pub border: Color32,
    pub danger: Color32,
    pub selection_bg: Color32,
}

impl GuiPopupTheme {
    pub fn dark() -> Self {
        Self {
            bg_elevated: Color32::from_rgb(30, 30, 30),
            bg_surface: Color32::from_rgb(24, 24, 24),
            bg_deep: Color32::from_rgb(18, 18, 18),
            bg_hover: Color32::from_rgb(40, 40, 40),
            fg_primary: Color32::from_rgb(255, 255, 255),
            fg_secondary: Color32::from_rgb(180, 180, 180),
            fg_dim: Color32::from_rgb(120, 120, 120),
            accent: Color32::from_rgb(34, 211, 238),
            accent_dim: Color32::from_rgb(6, 182, 212),
            border: Color32::from_rgb(60, 60, 60),
            danger: Color32::from_rgb(239, 68, 68),
            selection_bg: Color32::from_rgba_unmultiplied(34, 211, 238, 38),
        }
    }

    pub fn light() -> Self {
        Self {
            bg_elevated: Color32::from_rgb(255, 255, 255),
            bg_surface: Color32::from_rgb(245, 245, 245),
            bg_deep: Color32::from_rgb(235, 235, 235),
            bg_hover: Color32::from_rgb(225, 225, 225),
            fg_primary: Color32::from_rgb(0, 0, 0),
            fg_secondary: Color32::from_rgb(60, 60, 60),
            fg_dim: Color32::from_rgb(120, 120, 120),
            accent: Color32::from_rgb(6, 182, 212),
            accent_dim: Color32::from_rgb(8, 145, 178),
            border: Color32::from_rgb(200, 200, 200),
            danger: Color32::from_rgb(220, 38, 38),
            selection_bg: Color32::from_rgba_unmultiplied(6, 182, 212, 38),
        }
    }

    /// Create theme from individual color values
    /// Useful for converting from other theme types
    pub fn from_colors(
        bg_elevated: Color32,
        bg_surface: Color32,
        bg_deep: Color32,
        bg_hover: Color32,
        fg_primary: Color32,
        fg_secondary: Color32,
        fg_dim: Color32,
        accent: Color32,
        accent_dim: Color32,
        border: Color32,
        danger: Color32,
    ) -> Self {
        Self {
            bg_elevated,
            bg_surface,
            bg_deep,
            bg_hover,
            fg_primary,
            fg_secondary,
            fg_dim,
            accent,
            accent_dim,
            border,
            danger,
            selection_bg: Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 38),
        }
    }
}

/// Actions that can result from rendering
#[derive(Default)]
pub struct PopupActions {
    pub clicked_button: Option<super::ButtonId>,
    pub text_changed: Vec<(FieldId, String)>,
    pub toggle_changed: Vec<(FieldId, bool)>,
    pub select_changed: Vec<(FieldId, usize)>,
    pub number_changed: Vec<(FieldId, i64)>,
    pub list_selected: Vec<(FieldId, usize)>,
}

/// Render popup content inside a UI container
/// Returns actions that should be applied to state
pub fn render_popup_content(
    ui: &mut Ui,
    state: &PopupState,
    theme: &GuiPopupTheme,
    label_width: f32,
) -> PopupActions {
    let mut actions = PopupActions::default();

    let row_height = 28.0;
    let row_spacing = 8.0;

    // Render fields
    for field in &state.definition.fields {
        if !field.visible {
            continue;
        }

        let field_id = field.id;
        let is_selected = matches!(&state.selected, ElementSelection::Field(id) if *id == field_id);

        match &field.kind {
            FieldKind::Separator => {
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);
            }

            FieldKind::Label { text } => {
                ui.label(RichText::new(text).color(theme.fg_primary));
            }

            FieldKind::Text { value, masked, placeholder } => {
                let has_label = !field.label.is_empty();

                ui.horizontal(|ui| {
                    // Label (only if not empty)
                    if has_label {
                        ui.add_sized(
                            [label_width, row_height],
                            egui::Label::new(RichText::new(&field.label).color(theme.fg_secondary)),
                        );
                    }

                    // Text field
                    let display_value = if *masked {
                        "*".repeat(value.len())
                    } else {
                        value.clone()
                    };

                    let mut text = if state.editing && is_selected {
                        state.edit_buffer.clone()
                    } else {
                        display_value
                    };

                    let hint = placeholder.as_deref().unwrap_or("");

                    // For full-width fields (no label), add left margin
                    // Vertical margin calculated to center text in row_height (28px)
                    let horizontal_margin = if has_label { 4.0 } else { 15.0 };
                    let text_edit = egui::TextEdit::singleline(&mut text)
                        .hint_text(RichText::new(hint).color(theme.fg_dim))
                        .text_color(theme.fg_primary)
                        .frame(true)
                        .vertical_align(egui::Align::Center)
                        .margin(egui::vec2(horizontal_margin, 6.0));

                    let response = ui.add_sized(
                        [ui.available_width(), row_height],
                        text_edit,
                    );

                    if response.changed() {
                        actions.text_changed.push((field_id, text));
                    }
                });
                ui.add_space(row_spacing);
            }

            FieldKind::Toggle { value } => {
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [label_width, row_height],
                        egui::Label::new(RichText::new(&field.label).color(theme.fg_secondary)),
                    );

                    let mut checked = *value;
                    if ui.checkbox(&mut checked, "").changed() {
                        actions.toggle_changed.push((field_id, checked));
                    }
                });
                ui.add_space(row_spacing);
            }

            FieldKind::Select { options, selected_index } => {
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [label_width, row_height],
                        egui::Label::new(RichText::new(&field.label).color(theme.fg_secondary)),
                    );

                    let current = options.get(*selected_index).map(|o| o.label.as_str()).unwrap_or("-");
                    egui::ComboBox::from_id_source(format!("select_{}", field_id.0))
                        .selected_text(current)
                        .show_ui(ui, |ui| {
                            for (idx, opt) in options.iter().enumerate() {
                                let is_current = idx == *selected_index;
                                if ui.selectable_label(is_current, &opt.label).clicked() {
                                    actions.select_changed.push((field_id, idx));
                                }
                            }
                        });
                });
                ui.add_space(row_spacing);
            }

            FieldKind::Number { value, min, max } => {
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [label_width, row_height],
                        egui::Label::new(RichText::new(&field.label).color(theme.fg_secondary)),
                    );

                    let mut num = *value;
                    let range = match (min, max) {
                        (Some(lo), Some(hi)) => *lo..=*hi,
                        (Some(lo), None) => *lo..=i64::MAX,
                        (None, Some(hi)) => i64::MIN..=*hi,
                        (None, None) => i64::MIN..=i64::MAX,
                    };

                    if ui.add(egui::DragValue::new(&mut num).clamp_range(range)).changed() {
                        actions.number_changed.push((field_id, num));
                    }
                });
                ui.add_space(row_spacing);
            }

            FieldKind::List { items, selected_index, visible_height, headers, .. } => {
                // Auto-size columns based on content
                // First, measure max width of each column (except last which gets remaining space)
                let num_cols = items.first().map(|i| i.columns.len()).unwrap_or(3);
                let font_id = egui::TextStyle::Body.resolve(ui.style());
                let col_padding = 16.0; // padding between columns

                let mut col_widths: Vec<f32> = vec![0.0; num_cols];

                // Measure header widths
                if let Some(hdrs) = headers {
                    for (i, h) in hdrs.iter().enumerate() {
                        if i < num_cols {
                            let galley = ui.fonts(|f| f.layout_no_wrap(h.clone(), font_id.clone(), Color32::WHITE));
                            col_widths[i] = col_widths[i].max(galley.size().x + col_padding);
                        }
                    }
                }

                // Measure content widths for first N-1 columns (last column gets remaining space)
                for item in items.iter() {
                    for (i, col) in item.columns.iter().enumerate() {
                        if i < num_cols.saturating_sub(1) {
                            let galley = ui.fonts(|f| f.layout_no_wrap(col.clone(), font_id.clone(), Color32::WHITE));
                            col_widths[i] = col_widths[i].max(galley.size().x + col_padding);
                        }
                    }
                }

                // Calculate column start positions
                let mut col_starts: Vec<f32> = vec![0.0];
                for i in 0..num_cols.saturating_sub(1) {
                    col_starts.push(col_starts.last().unwrap() + col_widths[i]);
                }

                let available_width = ui.available_width();

                // Render headers
                if let Some(hdrs) = headers {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(available_width, row_height), egui::Sense::hover());
                    for (i, h) in hdrs.iter().enumerate() {
                        let x = rect.min.x + col_starts.get(i).copied().unwrap_or(0.0);
                        let text_pos = egui::pos2(x, rect.min.y + (row_height - font_id.size) / 2.0);
                        ui.painter().text(
                            text_pos,
                            egui::Align2::LEFT_TOP,
                            h.as_str(),
                            font_id.clone(),
                            theme.fg_dim,
                        );
                    }
                }

                // Calculate list height - use remaining available height minus space for buttons
                let min_list_height = (*visible_height as f32) * row_height;
                let available_height = ui.available_height() - 50.0; // Reserve space for buttons
                let list_height = available_height.max(min_list_height);

                // Draw slightly lighter background for list area
                let list_bg = Color32::from_rgb(38, 38, 38);
                egui::Frame::none()
                    .fill(list_bg)
                    .rounding(4.0)
                    .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .min_scrolled_height(list_height)
                    .max_height(list_height)
                    .auto_shrink([false, false])
                    .id_source(format!("{:?}_list", field_id))
                    .show(ui, |ui| {
                        let scroll_width = ui.available_width();
                        for (idx, item) in items.iter().enumerate() {
                            let is_item_selected = idx == *selected_index;

                            // Use explicit colors for list items
                            let text_color = if is_item_selected {
                                theme.fg_primary
                            } else {
                                theme.fg_secondary
                            };
                            let bg_color = if is_item_selected {
                                theme.selection_bg
                            } else {
                                Color32::TRANSPARENT
                            };

                            // Allocate row space
                            let (rect, response) = ui.allocate_exact_size(
                                egui::vec2(scroll_width, row_height),
                                egui::Sense::click()
                            );

                            // Draw selection background
                            if is_item_selected {
                                ui.painter().rect_filled(rect, 0.0, bg_color);
                            }

                            // Draw each column at fixed positions
                            for (i, col) in item.columns.iter().enumerate() {
                                let x = rect.min.x + col_starts.get(i).copied().unwrap_or(0.0);
                                let text_pos = egui::pos2(x, rect.min.y + (row_height - font_id.size) / 2.0);
                                ui.painter().text(
                                    text_pos,
                                    egui::Align2::LEFT_TOP,
                                    col,
                                    font_id.clone(),
                                    text_color,
                                );
                            }

                            if response.clicked() {
                                actions.list_selected.push((field_id, idx));
                            }
                        }
                    });
                }); // Close Frame
                ui.add_space(row_spacing);
            }

            FieldKind::ScrollableContent { lines, visible_height, .. } => {
                let list_height = (*visible_height as f32) * row_height;
                egui::ScrollArea::vertical()
                    .max_height(list_height)
                    .show(ui, |ui| {
                        for line in lines {
                            ui.label(RichText::new(line).color(theme.fg_primary));
                        }
                    });
            }

            FieldKind::MultilineText { value, visible_lines, .. } => {
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [label_width, row_height],
                        egui::Label::new(RichText::new(&field.label).color(theme.fg_secondary)),
                    );

                    let mut text = value.clone();
                    let response = ui.add_sized(
                        [ui.available_width(), (*visible_lines as f32) * row_height],
                        egui::TextEdit::multiline(&mut text)
                            .text_color(theme.fg_primary),
                    );

                    if response.changed() {
                        actions.text_changed.push((field_id, text));
                    }
                });
                ui.add_space(row_spacing);
            }
        }
    }

    // Render buttons - 15px gap between list and buttons
    if !state.definition.buttons.is_empty() {
        ui.add_space(15.0);

        // Split buttons: danger buttons on left, others on right
        let danger_buttons: Vec<_> = state.definition.buttons.iter()
            .filter(|b| b.enabled && matches!(b.style, ButtonStyle::Danger))
            .collect();
        let other_buttons: Vec<_> = state.definition.buttons.iter()
            .filter(|b| b.enabled && !matches!(b.style, ButtonStyle::Danger))
            .collect();

        ui.horizontal(|ui| {
            // Danger buttons on left
            for button in &danger_buttons {
                render_single_button(ui, button, state, theme, &mut actions.clicked_button);
                ui.add_space(8.0);
            }

            // Spacer to push other buttons to the right
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Render in reverse order for right-to-left layout
                for button in other_buttons.iter().rev() {
                    ui.add_space(8.0);
                    render_single_button(ui, button, state, theme, &mut actions.clicked_button);
                }
            });
        });
    }

    actions
}

fn render_single_button(
    ui: &mut Ui,
    button: &super::Button,
    state: &PopupState,
    theme: &GuiPopupTheme,
    clicked_button: &mut Option<super::ButtonId>,
) {
    let is_selected = matches!(&state.selected, ElementSelection::Button(id) if *id == button.id);

    let (fill, text_color) = match button.style {
        ButtonStyle::Primary => (theme.accent_dim, theme.bg_deep),
        ButtonStyle::Danger => (theme.danger, theme.fg_primary),
        ButtonStyle::Secondary => (theme.bg_hover, theme.fg_secondary),
    };

    let btn = egui::Button::new(
        RichText::new(&button.label)
            .color(text_color),
    )
    .fill(fill)
    .stroke(if is_selected {
        egui::Stroke::new(2.0, theme.accent)
    } else {
        egui::Stroke::NONE
    })
    .rounding(egui::Rounding::same(4.0))
    .min_size(egui::vec2(70.0, 28.0));

    if ui.add(btn).clicked() {
        *clicked_button = Some(button.id);
    }
}

#[allow(dead_code)]
fn render_buttons(
    ui: &mut Ui,
    state: &PopupState,
    theme: &GuiPopupTheme,
    clicked_button: &mut Option<super::ButtonId>,
) {
    let buttons: Vec<_> = state.definition.buttons.iter().filter(|b| b.enabled).collect();

    // Render in reverse order for right-to-left layout
    for button in buttons.iter().rev() {
        render_single_button(ui, button, state, theme, clicked_button);
        ui.add_space(8.0);
    }
}

/// Apply actions from rendering to popup state
pub fn apply_actions(state: &mut PopupState, actions: PopupActions) {
    // Apply text changes
    for (field_id, text) in actions.text_changed {
        if !state.editing {
            state.start_edit();
        }
        state.edit_buffer = text.clone();
        // Also update the field value directly for GUI
        if let Some(field) = state.field_mut(field_id) {
            field.kind.set_text(text);
        }
    }

    // Apply toggle changes
    for (field_id, value) in actions.toggle_changed {
        if let Some(field) = state.field_mut(field_id) {
            if let FieldKind::Toggle { value: v } = &mut field.kind {
                *v = value;
            }
        }
    }

    // Apply select changes
    for (field_id, idx) in actions.select_changed {
        if let Some(field) = state.field_mut(field_id) {
            if let FieldKind::Select { selected_index, .. } = &mut field.kind {
                *selected_index = idx;
            }
        }
    }

    // Apply number changes
    for (field_id, value) in actions.number_changed {
        if let Some(field) = state.field_mut(field_id) {
            field.kind.set_number(value);
        }
    }

    // Apply list selection changes
    for (field_id, idx) in actions.list_selected {
        if let Some(field) = state.field_mut(field_id) {
            if let FieldKind::List { selected_index, .. } = &mut field.kind {
                *selected_index = idx;
            }
        }
    }
}

/// Handle keyboard input for popup navigation
/// Returns true if the key was handled
pub fn handle_popup_key(state: &mut PopupState, key: egui::Key, modifiers: &egui::Modifiers) -> bool {
    match key {
        egui::Key::ArrowUp => {
            if state.editing {
                state.commit_edit();
            }
            if state.is_on_button() {
                state.select_last_field();
            } else {
                state.prev_field();
            }
            // Auto-start editing for text fields
            if state.selected_field().map(|f| f.kind.is_text()).unwrap_or(false) {
                state.start_edit();
            }
            true
        }
        egui::Key::ArrowDown => {
            if state.editing {
                state.commit_edit();
            }
            if state.is_on_button() {
                state.select_last_field();
            } else {
                state.next_field();
            }
            // Auto-start editing for text fields
            if state.selected_field().map(|f| f.kind.is_text()).unwrap_or(false) {
                state.start_edit();
            }
            true
        }
        egui::Key::ArrowLeft => {
            if state.editing {
                state.cursor_left();
            } else {
                state.decrease_current();
            }
            true
        }
        egui::Key::ArrowRight => {
            if state.editing {
                state.cursor_right();
            } else {
                state.increase_current();
            }
            true
        }
        egui::Key::Tab => {
            if state.editing {
                state.commit_edit();
            }
            if modifiers.shift {
                if state.is_on_button() {
                    state.prev_button();
                } else {
                    state.select_last_field();
                }
            } else if state.is_on_button() {
                state.next_button();
            } else {
                state.select_first_button();
            }
            true
        }
        egui::Key::Enter => {
            if state.editing {
                state.commit_edit();
            } else if state.is_on_field() {
                state.toggle_current();
            }
            true
        }
        egui::Key::Space => {
            if !state.editing {
                state.toggle_current();
            }
            true
        }
        egui::Key::Home => {
            if state.editing {
                state.cursor_home();
                true
            } else {
                false
            }
        }
        egui::Key::End => {
            if state.editing {
                state.cursor_end();
                true
            } else {
                false
            }
        }
        _ => false,
    }
}

/// Result from showing a popup in a viewport
pub struct ViewportPopupResult {
    /// Button that was clicked (if any)
    pub clicked_button: Option<super::ButtonId>,
    /// Whether the popup should be closed
    pub should_close: bool,
    /// Actions to apply to the popup state
    pub actions: PopupActions,
}

/// Show a popup in a separate viewport window
/// Returns the result including any button clicks and whether to close
pub fn show_popup_viewport(
    ctx: &egui::Context,
    state: &PopupState,
    theme: &GuiPopupTheme,
    viewport_id: &str,
    size: [f32; 2],
) -> ViewportPopupResult {
    let mut result = ViewportPopupResult {
        clicked_button: None,
        should_close: false,
        actions: PopupActions::default(),
    };

    ctx.show_viewport_immediate(
        egui::ViewportId::from_hash_of(viewport_id),
        egui::ViewportBuilder::default()
            .with_title(&state.definition.title)
            .with_inner_size(size),
        |ctx, _class| {
            // Apply popup styling
            ctx.style_mut(|style| {
                style.visuals.window_fill = theme.bg_elevated;
                style.visuals.panel_fill = theme.bg_elevated;
                style.visuals.window_stroke = egui::Stroke::NONE;
                style.visuals.window_shadow = egui::epaint::Shadow::NONE;

                let widget_bg = theme.bg_deep;
                let widget_rounding = egui::Rounding::same(4.0);

                style.visuals.widgets.noninteractive.bg_fill = widget_bg;
                style.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::NONE;
                style.visuals.widgets.noninteractive.fg_stroke = egui::Stroke::NONE;
                style.visuals.widgets.noninteractive.rounding = widget_rounding;
                style.visuals.widgets.noninteractive.weak_bg_fill = widget_bg;

                style.visuals.widgets.inactive.bg_fill = theme.bg_hover;
                style.visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
                style.visuals.widgets.inactive.fg_stroke = egui::Stroke::NONE;
                style.visuals.widgets.inactive.rounding = widget_rounding;
                style.visuals.widgets.inactive.weak_bg_fill = widget_bg;

                style.visuals.widgets.hovered.bg_fill = theme.bg_hover;
                style.visuals.widgets.hovered.bg_stroke = egui::Stroke::NONE;
                style.visuals.widgets.hovered.fg_stroke = egui::Stroke::NONE;
                style.visuals.widgets.hovered.rounding = widget_rounding;
                style.visuals.widgets.hovered.weak_bg_fill = widget_bg;

                style.visuals.widgets.active.bg_fill = theme.accent_dim;
                style.visuals.widgets.active.bg_stroke = egui::Stroke::NONE;
                style.visuals.widgets.active.fg_stroke = egui::Stroke::NONE;
                style.visuals.widgets.active.rounding = widget_rounding;
                style.visuals.widgets.active.weak_bg_fill = widget_bg;

                style.visuals.widgets.open.bg_fill = widget_bg;
                style.visuals.widgets.open.bg_stroke = egui::Stroke::NONE;
                style.visuals.widgets.open.fg_stroke = egui::Stroke::NONE;
                style.visuals.widgets.open.rounding = widget_rounding;
                style.visuals.widgets.open.weak_bg_fill = widget_bg;

                style.visuals.selection.bg_fill = theme.selection_bg;
                style.visuals.selection.stroke = egui::Stroke::NONE;
                style.visuals.extreme_bg_color = widget_bg;
            });

            // Handle escape key and close request
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) ||
               ctx.input(|i| i.viewport().close_requested()) {
                result.should_close = true;
            }

            // Render popup content
            egui::CentralPanel::default()
                .frame(egui::Frame::none()
                    .fill(theme.bg_elevated)
                    .inner_margin(egui::Margin { left: 20.0, right: 16.0, top: 20.0, bottom: 16.0 }))
                .show(ctx, |ui| {
                    let label_width = state.definition.layout.label_width as f32 * 8.0; // Approximate conversion
                    result.actions = render_popup_content(ui, state, theme, label_width);
                    result.clicked_button = result.actions.clicked_button;
                });
        },
    );

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_theme_creation() {
        let dark = GuiPopupTheme::dark();
        let light = GuiPopupTheme::light();

        // Dark theme should have dark background
        assert!(dark.bg_elevated.r() < 50);
        // Light theme should have light background
        assert!(light.bg_elevated.r() > 200);
    }
}
