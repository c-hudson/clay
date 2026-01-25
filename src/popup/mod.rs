//! Unified popup/window management system
//!
//! This module provides a single data model for popups that can be rendered
//! by console (ratatui), GUI (egui), and web (JavaScript) interfaces.

pub mod console_renderer;
pub mod definitions;

#[cfg(feature = "remote-gui")]
pub mod gui_renderer;

use std::collections::HashMap;

// ============================================================================
// Field and Button IDs
// ============================================================================

/// Type-safe field identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FieldId(pub u32);

/// Type-safe button identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ButtonId(pub u32);

/// Type-safe popup identifier
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PopupId(pub &'static str);

// ============================================================================
// Field Types
// ============================================================================

/// Option for Select fields
#[derive(Debug, Clone)]
pub struct SelectOption {
    pub value: String,
    pub label: String,
}

impl SelectOption {
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
        }
    }

    /// Create an option where value and label are the same
    pub fn simple(value: impl Into<String>) -> Self {
        let v = value.into();
        Self {
            label: v.clone(),
            value: v,
        }
    }
}

/// Different kinds of form fields
#[derive(Debug, Clone)]
pub enum FieldKind {
    /// Text input field
    Text {
        value: String,
        masked: bool,
        placeholder: Option<String>,
    },
    /// Boolean toggle
    Toggle { value: bool },
    /// Selection from a list of options
    Select {
        options: Vec<SelectOption>,
        selected_index: usize,
    },
    /// Numeric input
    Number {
        value: i64,
        min: Option<i64>,
        max: Option<i64>,
    },
    /// Static label (read-only)
    Label { text: String },
    /// Visual separator line
    Separator,
    /// Multi-line text editor
    MultilineText {
        value: String,
        line_count: usize,
    },
    /// List of items (for selection popups)
    List {
        items: Vec<ListItem>,
        selected_index: usize,
        scroll_offset: usize,
        visible_height: usize,
        /// Column headers (optional)
        headers: Option<Vec<String>>,
        /// Fixed column widths (optional, prevents resizing when filtering)
        column_widths: Option<Vec<usize>>,
    },
    /// Scrollable read-only content (for help, large text display)
    ScrollableContent {
        lines: Vec<String>,
        scroll_offset: usize,
        visible_height: usize,
    },
}

/// An item in a list field
#[derive(Debug, Clone)]
pub struct ListItem {
    pub id: String,
    pub columns: Vec<String>,
    pub style: ListItemStyle,
}

/// Styling hints for list items
#[derive(Debug, Clone, Copy, Default)]
pub struct ListItemStyle {
    pub is_current: bool,
    pub is_connected: bool,
    pub is_disabled: bool,
}

impl FieldKind {
    /// Create a text field
    pub fn text(value: impl Into<String>) -> Self {
        Self::Text {
            value: value.into(),
            masked: false,
            placeholder: None,
        }
    }

    /// Create a masked (password) text field
    pub fn password(value: impl Into<String>) -> Self {
        Self::Text {
            value: value.into(),
            masked: true,
            placeholder: None,
        }
    }

    /// Create a text field with placeholder
    pub fn text_with_placeholder(value: impl Into<String>, placeholder: impl Into<String>) -> Self {
        Self::Text {
            value: value.into(),
            masked: false,
            placeholder: Some(placeholder.into()),
        }
    }

    /// Create a toggle field
    pub fn toggle(value: bool) -> Self {
        Self::Toggle { value }
    }

    /// Create a select field
    pub fn select(options: Vec<SelectOption>, selected_index: usize) -> Self {
        Self::Select {
            options,
            selected_index,
        }
    }

    /// Create a number field
    pub fn number(value: i64) -> Self {
        Self::Number {
            value,
            min: None,
            max: None,
        }
    }

    /// Create a number field with range
    pub fn number_range(value: i64, min: i64, max: i64) -> Self {
        Self::Number {
            value,
            min: Some(min),
            max: Some(max),
        }
    }

    /// Create a label
    pub fn label(text: impl Into<String>) -> Self {
        Self::Label { text: text.into() }
    }

    /// Create a separator
    pub fn separator() -> Self {
        Self::Separator
    }

    /// Create a multiline text field
    pub fn multiline(value: impl Into<String>, line_count: usize) -> Self {
        Self::MultilineText {
            value: value.into(),
            line_count,
        }
    }

    /// Create a list field
    pub fn list(items: Vec<ListItem>, visible_height: usize) -> Self {
        Self::List {
            items,
            selected_index: 0,
            scroll_offset: 0,
            visible_height,
            headers: None,
            column_widths: None,
        }
    }

    /// Create a list field with column headers
    pub fn list_with_headers(items: Vec<ListItem>, visible_height: usize, headers: &[&str]) -> Self {
        Self::List {
            items,
            selected_index: 0,
            scroll_offset: 0,
            visible_height,
            headers: Some(headers.iter().map(|s| s.to_string()).collect()),
            column_widths: None,
        }
    }

    /// Create a list field with column headers and fixed column widths
    pub fn list_with_headers_and_widths(items: Vec<ListItem>, visible_height: usize, headers: &[&str], column_widths: Vec<usize>) -> Self {
        Self::List {
            items,
            selected_index: 0,
            scroll_offset: 0,
            visible_height,
            headers: Some(headers.iter().map(|s| s.to_string()).collect()),
            column_widths: Some(column_widths),
        }
    }

    /// Create a scrollable content field
    pub fn scrollable_content(lines: Vec<String>, visible_height: usize) -> Self {
        Self::ScrollableContent {
            lines,
            scroll_offset: 0,
            visible_height,
        }
    }

    /// Create a scrollable content field from static string slices
    pub fn scrollable_content_static(lines: &[&str], visible_height: usize) -> Self {
        Self::ScrollableContent {
            lines: lines.iter().map(|s| s.to_string()).collect(),
            scroll_offset: 0,
            visible_height,
        }
    }

    /// Get the string value for text-like fields
    pub fn get_text(&self) -> Option<&str> {
        match self {
            Self::Text { value, .. } => Some(value),
            Self::MultilineText { value, .. } => Some(value),
            _ => None,
        }
    }

    /// Check if this is a text-like field (supports text editing)
    pub fn is_text(&self) -> bool {
        matches!(self, Self::Text { .. } | Self::MultilineText { .. })
    }

    /// Set the string value for text-like fields
    pub fn set_text(&mut self, new_value: String) {
        match self {
            Self::Text { value, .. } => *value = new_value,
            Self::MultilineText { value, .. } => *value = new_value,
            _ => {}
        }
    }

    /// Get the boolean value for toggle fields
    pub fn get_bool(&self) -> Option<bool> {
        match self {
            Self::Toggle { value } => Some(*value),
            _ => None,
        }
    }

    /// Toggle a boolean field
    pub fn toggle_bool(&mut self) {
        if let Self::Toggle { value } = self {
            *value = !*value;
        }
    }

    /// Get selected value for select fields
    pub fn get_selected(&self) -> Option<&str> {
        match self {
            Self::Select { options, selected_index } => {
                options.get(*selected_index).map(|o| o.value.as_str())
            }
            _ => None,
        }
    }

    /// Cycle to next option in select field
    pub fn cycle_next(&mut self) {
        if let Self::Select { options, selected_index } = self {
            if !options.is_empty() {
                *selected_index = (*selected_index + 1) % options.len();
            }
        }
    }

    /// Cycle to previous option in select field
    pub fn cycle_prev(&mut self) {
        if let Self::Select { options, selected_index } = self {
            if !options.is_empty() {
                *selected_index = if *selected_index == 0 {
                    options.len() - 1
                } else {
                    *selected_index - 1
                };
            }
        }
    }

    /// Get numeric value
    pub fn get_number(&self) -> Option<i64> {
        match self {
            Self::Number { value, .. } => Some(*value),
            _ => None,
        }
    }

    /// Set numeric value (respects min/max if set)
    pub fn set_number(&mut self, new_value: i64) {
        if let Self::Number { value, min, max } = self {
            let clamped = match (*min, *max) {
                (Some(lo), Some(hi)) => new_value.clamp(lo, hi),
                (Some(lo), None) => new_value.max(lo),
                (None, Some(hi)) => new_value.min(hi),
                (None, None) => new_value,
            };
            *value = clamped;
        }
    }

    /// Increment number field
    pub fn increment(&mut self) {
        if let Some(v) = self.get_number() {
            self.set_number(v + 1);
        }
    }

    /// Decrement number field
    pub fn decrement(&mut self) {
        if let Some(v) = self.get_number() {
            self.set_number(v - 1);
        }
    }

    /// Check if this field kind is interactive (can be edited/toggled)
    pub fn is_interactive(&self) -> bool {
        !matches!(self, Self::Label { .. } | Self::Separator)
    }

    /// Check if this is a text-editable field
    pub fn is_text_editable(&self) -> bool {
        matches!(self, Self::Text { .. } | Self::MultilineText { .. })
    }
}

// ============================================================================
// Field Definition
// ============================================================================

/// A form field definition
#[derive(Debug, Clone)]
pub struct Field {
    pub id: FieldId,
    pub label: String,
    pub kind: FieldKind,
    pub visible: bool,
    pub enabled: bool,
    /// Keyboard shortcut to select this field (like button shortcuts)
    pub shortcut: Option<char>,
}

impl Field {
    pub fn new(id: FieldId, label: impl Into<String>, kind: FieldKind) -> Self {
        Self {
            id,
            label: label.into(),
            kind,
            visible: true,
            enabled: true,
            shortcut: None,
        }
    }

    /// Create an invisible field (not rendered but holds state)
    pub fn hidden(id: FieldId, kind: FieldKind) -> Self {
        Self {
            id,
            label: String::new(),
            kind,
            visible: false,
            enabled: false,
            shortcut: None,
        }
    }

    /// Create a disabled field
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    /// Add a keyboard shortcut to select this field
    pub fn with_shortcut(mut self, shortcut: char) -> Self {
        self.shortcut = Some(shortcut);
        self
    }

    /// Check if this field can receive focus
    pub fn is_focusable(&self) -> bool {
        self.visible && self.enabled && self.kind.is_interactive()
    }
}

// ============================================================================
// Button Definition
// ============================================================================

/// Button styling hints
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ButtonStyle {
    /// Default/secondary button
    #[default]
    Secondary,
    /// Primary action button (e.g., Save)
    Primary,
    /// Destructive action (e.g., Delete)
    Danger,
}

/// A button in a popup
#[derive(Debug, Clone)]
pub struct Button {
    pub id: ButtonId,
    pub label: String,
    pub shortcut: Option<char>,
    pub style: ButtonStyle,
    pub enabled: bool,
}

impl Button {
    pub fn new(id: ButtonId, label: impl Into<String>) -> Self {
        Self {
            id,
            label: label.into(),
            shortcut: None,
            style: ButtonStyle::Secondary,
            enabled: true,
        }
    }

    pub fn primary(mut self) -> Self {
        self.style = ButtonStyle::Primary;
        self
    }

    pub fn danger(mut self) -> Self {
        self.style = ButtonStyle::Danger;
        self
    }

    pub fn with_shortcut(mut self, shortcut: char) -> Self {
        self.shortcut = Some(shortcut);
        self
    }

    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }
}

// ============================================================================
// Popup Layout
// ============================================================================

/// Layout configuration for a popup
#[derive(Debug, Clone)]
pub struct PopupLayout {
    /// Width of label column in characters
    pub label_width: usize,
    /// Minimum width of the popup content area
    pub min_width: usize,
    /// Maximum width (as percentage of screen, 0 = no limit)
    pub max_width_percent: usize,
    /// Whether to center the popup horizontally
    pub center_horizontal: bool,
    /// Whether to center the popup vertically
    pub center_vertical: bool,
    /// Whether the popup is modal (blocks input to background)
    pub modal: bool,
    /// Whether to right-align buttons (default: centered)
    pub buttons_right_align: bool,
    /// Whether to add a blank line before list fields
    pub blank_line_before_list: bool,
}

impl Default for PopupLayout {
    fn default() -> Self {
        Self {
            label_width: 12,
            min_width: 40,
            max_width_percent: 80,
            center_horizontal: true,
            center_vertical: true,
            modal: true,
            buttons_right_align: false,
            blank_line_before_list: false,
        }
    }
}

impl PopupLayout {
    pub fn small() -> Self {
        Self {
            label_width: 10,
            min_width: 30,
            max_width_percent: 50,
            ..Default::default()
        }
    }

    pub fn medium() -> Self {
        Self::default()
    }

    pub fn large() -> Self {
        Self {
            label_width: 14,
            min_width: 60,
            max_width_percent: 90,
            ..Default::default()
        }
    }

    pub fn full_width() -> Self {
        Self {
            label_width: 14,
            min_width: 0,
            max_width_percent: 95,
            ..Default::default()
        }
    }
}

// ============================================================================
// Popup Definition
// ============================================================================

/// Static definition of a popup's structure
#[derive(Debug, Clone)]
pub struct PopupDefinition {
    pub id: PopupId,
    pub title: String,
    pub fields: Vec<Field>,
    pub buttons: Vec<Button>,
    pub layout: PopupLayout,
    /// Custom key-value data for app-specific context (e.g., world index for delete confirm)
    pub custom_data: std::collections::HashMap<String, String>,
}

impl PopupDefinition {
    pub fn new(id: PopupId, title: impl Into<String>) -> Self {
        Self {
            id,
            title: title.into(),
            fields: Vec::new(),
            buttons: Vec::new(),
            layout: PopupLayout::default(),
            custom_data: std::collections::HashMap::new(),
        }
    }

    pub fn with_field(mut self, field: Field) -> Self {
        self.fields.push(field);
        self
    }

    pub fn with_button(mut self, button: Button) -> Self {
        self.buttons.push(button);
        self
    }

    pub fn with_layout(mut self, layout: PopupLayout) -> Self {
        self.layout = layout;
        self
    }

    /// Get a field by ID
    pub fn get_field(&self, id: FieldId) -> Option<&Field> {
        self.fields.iter().find(|f| f.id == id)
    }

    /// Get a mutable field by ID
    pub fn get_field_mut(&mut self, id: FieldId) -> Option<&mut Field> {
        self.fields.iter_mut().find(|f| f.id == id)
    }

    /// Get a button by ID
    pub fn get_button(&self, id: ButtonId) -> Option<&Button> {
        self.buttons.iter().find(|b| b.id == id)
    }

    /// Get focusable field IDs in order
    pub fn focusable_fields(&self) -> Vec<FieldId> {
        self.fields
            .iter()
            .filter(|f| f.is_focusable())
            .map(|f| f.id)
            .collect()
    }

    /// Get enabled button IDs in order
    pub fn enabled_buttons(&self) -> Vec<ButtonId> {
        self.buttons
            .iter()
            .filter(|b| b.enabled)
            .map(|b| b.id)
            .collect()
    }
}

// ============================================================================
// Selection State
// ============================================================================

/// What element is currently selected in the popup
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElementSelection {
    /// A field is selected
    Field(FieldId),
    /// A button is selected
    Button(ButtonId),
    /// Nothing is selected
    None,
}

// ============================================================================
// Popup State (Runtime)
// ============================================================================

/// Runtime state for a popup
#[derive(Debug, Clone)]
pub struct PopupState {
    /// The popup definition
    pub definition: PopupDefinition,
    /// Whether the popup is visible
    pub visible: bool,
    /// Currently selected element
    pub selected: ElementSelection,
    /// Whether we're in text editing mode
    pub editing: bool,
    /// Cursor position within the edit buffer
    pub edit_cursor: usize,
    /// Horizontal scroll offset for long text fields
    pub edit_scroll: usize,
    /// Current edit buffer (copy of field value while editing)
    pub edit_buffer: String,
    /// Error message to display
    pub error: Option<String>,
    /// Scroll offset for scrollable content
    pub scroll_offset: usize,
    /// Custom state for complex popups (e.g., filter text)
    pub custom: HashMap<String, String>,
    /// Actual rendered content height (set during rendering, used for scroll calculations)
    pub actual_content_height: Option<usize>,
}

impl PopupState {
    /// Create a new popup state from a definition
    pub fn new(definition: PopupDefinition) -> Self {
        // Select the first focusable element
        let selected = definition
            .focusable_fields()
            .first()
            .map(|id| ElementSelection::Field(*id))
            .or_else(|| {
                definition
                    .enabled_buttons()
                    .first()
                    .map(|id| ElementSelection::Button(*id))
            })
            .unwrap_or(ElementSelection::None);

        Self {
            definition,
            visible: false,
            selected,
            editing: false,
            edit_cursor: 0,
            edit_scroll: 0,
            edit_buffer: String::new(),
            error: None,
            scroll_offset: 0,
            custom: HashMap::new(),
            actual_content_height: None,
        }
    }

    /// Open the popup
    pub fn open(&mut self) {
        self.visible = true;
        self.error = None;
        // Reset to first focusable element
        self.selected = self.definition
            .focusable_fields()
            .first()
            .map(|id| ElementSelection::Field(*id))
            .or_else(|| {
                self.definition
                    .enabled_buttons()
                    .first()
                    .map(|id| ElementSelection::Button(*id))
            })
            .unwrap_or(ElementSelection::None);
    }

    /// Close the popup
    pub fn close(&mut self) {
        self.visible = false;
        self.editing = false;
        self.error = None;
    }

    /// Get the currently selected field (if any)
    pub fn selected_field(&self) -> Option<&Field> {
        if let ElementSelection::Field(id) = &self.selected {
            self.definition.get_field(*id)
        } else {
            None
        }
    }

    /// Get the currently selected field mutably (if any)
    pub fn selected_field_mut(&mut self) -> Option<&mut Field> {
        if let ElementSelection::Field(id) = &self.selected {
            let id = *id;
            self.definition.get_field_mut(id)
        } else {
            None
        }
    }

    /// Get the currently selected button (if any)
    pub fn selected_button(&self) -> Option<&Button> {
        if let ElementSelection::Button(id) = &self.selected {
            self.definition.get_button(*id)
        } else {
            None
        }
    }

    /// Get a field by ID
    pub fn field(&self, id: FieldId) -> Option<&Field> {
        self.definition.get_field(id)
    }

    /// Get a mutable field by ID
    pub fn field_mut(&mut self, id: FieldId) -> Option<&mut Field> {
        self.definition.get_field_mut(id)
    }

    /// Get a field value as string
    pub fn get_text(&self, id: FieldId) -> Option<&str> {
        self.definition.get_field(id).and_then(|f| f.kind.get_text())
    }

    /// Get a field value as bool
    pub fn get_bool(&self, id: FieldId) -> Option<bool> {
        self.definition.get_field(id).and_then(|f| f.kind.get_bool())
    }

    /// Get a field's selected value
    pub fn get_selected(&self, id: FieldId) -> Option<&str> {
        self.definition.get_field(id).and_then(|f| f.kind.get_selected())
    }

    /// Get a field value as number
    pub fn get_number(&self, id: FieldId) -> Option<i64> {
        self.definition.get_field(id).and_then(|f| f.kind.get_number())
    }

    /// Set a text field value
    pub fn set_text(&mut self, id: FieldId, value: String) {
        if let Some(field) = self.definition.get_field_mut(id) {
            field.kind.set_text(value);
        }
    }

    // ========================================================================
    // Navigation
    // ========================================================================

    /// Move to the next focusable field (does not wrap to buttons)
    /// Returns true if moved, false if at edge
    pub fn next_field(&mut self) -> bool {
        if let ElementSelection::Field(current_id) = &self.selected {
            let fields = self.definition.focusable_fields();
            if let Some(idx) = fields.iter().position(|id| id == current_id) {
                if idx + 1 < fields.len() {
                    self.selected = ElementSelection::Field(fields[idx + 1]);
                    return true;
                }
            }
        }
        false
    }

    /// Move to the previous focusable field (does not wrap)
    /// Returns true if moved, false if at edge
    pub fn prev_field(&mut self) -> bool {
        if let ElementSelection::Field(current_id) = &self.selected {
            let fields = self.definition.focusable_fields();
            if let Some(idx) = fields.iter().position(|id| id == current_id) {
                if idx > 0 {
                    self.selected = ElementSelection::Field(fields[idx - 1]);
                    return true;
                }
            }
        }
        false
    }

    /// Move to the next button (cycles within buttons only)
    pub fn next_button(&mut self) {
        let buttons = self.definition.enabled_buttons();
        if buttons.is_empty() {
            return;
        }

        let next_idx = if let ElementSelection::Button(current_id) = &self.selected {
            buttons
                .iter()
                .position(|id| id == current_id)
                .map(|idx| (idx + 1) % buttons.len())
                .unwrap_or(0)
        } else {
            0
        };

        self.selected = ElementSelection::Button(buttons[next_idx]);
    }

    /// Move to the previous button (cycles within buttons only)
    pub fn prev_button(&mut self) {
        let buttons = self.definition.enabled_buttons();
        if buttons.is_empty() {
            return;
        }

        let prev_idx = if let ElementSelection::Button(current_id) = &self.selected {
            buttons
                .iter()
                .position(|id| id == current_id)
                .map(|idx| if idx == 0 { buttons.len() - 1 } else { idx - 1 })
                .unwrap_or(buttons.len() - 1)
        } else {
            buttons.len() - 1
        };

        self.selected = ElementSelection::Button(buttons[prev_idx]);
    }

    /// Jump to first button (for Tab key)
    pub fn select_first_button(&mut self) {
        if let Some(id) = self.definition.enabled_buttons().first() {
            self.selected = ElementSelection::Button(*id);
        }
    }

    /// Select a specific field
    pub fn select_field(&mut self, id: FieldId) {
        if self.definition.get_field(id).map(|f| f.is_focusable()).unwrap_or(false) {
            self.selected = ElementSelection::Field(id);
        }
    }

    /// Select a specific button
    pub fn select_button(&mut self, id: ButtonId) {
        if self.definition.get_button(id).map(|b| b.enabled).unwrap_or(false) {
            self.selected = ElementSelection::Button(id);
        }
    }

    /// Select the last focusable field
    pub fn select_last_field(&mut self) {
        let fields = self.definition.focusable_fields();
        if let Some(id) = fields.last() {
            self.selected = ElementSelection::Field(*id);
        }
    }

    /// Toggle the current field's value (for boolean and select fields)
    pub fn toggle_current(&mut self) {
        if let ElementSelection::Field(id) = &self.selected {
            let id = *id;
            if let Some(field) = self.definition.get_field_mut(id) {
                match &mut field.kind {
                    FieldKind::Toggle { value } => {
                        *value = !*value;
                    }
                    FieldKind::Select { options, selected_index } => {
                        if !options.is_empty() {
                            *selected_index = (*selected_index + 1) % options.len();
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// Increase the current field's value (number +1 or select next)
    pub fn increase_current(&mut self) {
        if let ElementSelection::Field(id) = &self.selected {
            let id = *id;
            if let Some(field) = self.definition.get_field_mut(id) {
                match &mut field.kind {
                    FieldKind::Number { value, max, .. } => {
                        if let Some(m) = max {
                            if *value < *m {
                                *value += 1;
                            }
                        } else {
                            *value += 1;
                        }
                    }
                    FieldKind::Select { options, selected_index } => {
                        if !options.is_empty() {
                            *selected_index = (*selected_index + 1) % options.len();
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// Decrease the current field's value (number -1 or select prev)
    pub fn decrease_current(&mut self) {
        if let ElementSelection::Field(id) = &self.selected {
            let id = *id;
            if let Some(field) = self.definition.get_field_mut(id) {
                match &mut field.kind {
                    FieldKind::Number { value, min, .. } => {
                        if let Some(m) = min {
                            if *value > *m {
                                *value -= 1;
                            }
                        } else {
                            *value -= 1;
                        }
                    }
                    FieldKind::Select { options, selected_index } => {
                        if !options.is_empty() {
                            if *selected_index == 0 {
                                *selected_index = options.len() - 1;
                            } else {
                                *selected_index -= 1;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// Check if currently on a field
    pub fn is_on_field(&self) -> bool {
        matches!(self.selected, ElementSelection::Field(_))
    }

    /// Check if currently on a button
    pub fn is_on_button(&self) -> bool {
        matches!(self.selected, ElementSelection::Button(_))
    }

    /// Check if a specific button is focused
    pub fn is_button_focused(&self, id: ButtonId) -> bool {
        matches!(&self.selected, ElementSelection::Button(selected_id) if *selected_id == id)
    }

    /// Cycle between focusable text fields and buttons (for Tab key)
    /// Cycles: filter field -> button1 -> button2 -> ... -> buttonN -> filter field
    /// Returns true if selection changed
    pub fn cycle_field_buttons(&mut self) -> bool {
        let buttons = self.definition.enabled_buttons();

        // Find text fields that can be cycled to
        let text_fields: Vec<FieldId> = self.definition.focusable_fields()
            .iter()
            .filter(|id| {
                self.definition.get_field(**id)
                    .map(|f| f.kind.is_text_editable())
                    .unwrap_or(false)
            })
            .copied()
            .collect();

        if text_fields.is_empty() && buttons.is_empty() {
            return false;
        }

        match &self.selected {
            ElementSelection::Field(_) => {
                // If on any field (including List), go to first button
                if let Some(btn) = buttons.first() {
                    self.selected = ElementSelection::Button(*btn);
                    return true;
                }
            }
            ElementSelection::Button(current_id) => {
                // If on a button, go to next button, or wrap to first text field
                if let Some(idx) = buttons.iter().position(|id| id == current_id) {
                    if idx + 1 < buttons.len() {
                        // Go to next button
                        self.selected = ElementSelection::Button(buttons[idx + 1]);
                        return true;
                    } else {
                        // On last button, wrap to first text field
                        if let Some(field_id) = text_fields.first() {
                            self.selected = ElementSelection::Field(*field_id);
                            return true;
                        }
                    }
                }
            }
            ElementSelection::None => {
                // Select first available
                if let Some(field_id) = text_fields.first() {
                    self.selected = ElementSelection::Field(*field_id);
                    return true;
                } else if let Some(btn) = buttons.first() {
                    self.selected = ElementSelection::Button(*btn);
                    return true;
                }
            }
        }
        false
    }

    /// Find and select a field by its shortcut key
    /// Returns true if a field was selected
    pub fn select_field_by_shortcut(&mut self, key: char) -> bool {
        let key_lower = key.to_ascii_lowercase();
        for field in &self.definition.fields {
            if field.is_focusable() {
                if let Some(shortcut) = field.shortcut {
                    if shortcut.to_ascii_lowercase() == key_lower {
                        self.selected = ElementSelection::Field(field.id);
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Check if currently editing a text field
    pub fn is_editing_text(&self) -> bool {
        self.editing && self.selected_field().map(|f| f.kind.is_text_editable()).unwrap_or(false)
    }

    /// Check if a specific field is selected
    pub fn is_field_selected(&self, id: FieldId) -> bool {
        matches!(&self.selected, ElementSelection::Field(selected_id) if *selected_id == id)
    }

    // ========================================================================
    // Text Editing
    // ========================================================================

    /// Start editing the currently selected text field
    pub fn start_edit(&mut self) {
        if let Some(field) = self.selected_field() {
            if let Some(text) = field.kind.get_text() {
                self.edit_buffer = text.to_string();
                self.edit_cursor = self.edit_buffer.len();
                self.edit_scroll = 0;
                self.editing = true;
            }
        }
    }

    /// Commit the current edit
    pub fn commit_edit(&mut self) {
        if self.editing {
            if let ElementSelection::Field(id) = &self.selected {
                let id = *id;
                if let Some(field) = self.definition.get_field_mut(id) {
                    field.kind.set_text(self.edit_buffer.clone());
                }
            }
            self.editing = false;
        }
    }

    /// Cancel the current edit
    pub fn cancel_edit(&mut self) {
        self.editing = false;
    }

    /// Insert a character at cursor
    pub fn insert_char(&mut self, c: char) {
        if self.editing {
            let byte_pos = self.edit_buffer
                .char_indices()
                .nth(self.edit_cursor)
                .map(|(i, _)| i)
                .unwrap_or(self.edit_buffer.len());
            self.edit_buffer.insert(byte_pos, c);
            self.edit_cursor += 1;
        }
    }

    /// Delete character before cursor (backspace)
    pub fn backspace(&mut self) {
        if self.editing && self.edit_cursor > 0 {
            let char_indices: Vec<_> = self.edit_buffer.char_indices().collect();
            if self.edit_cursor <= char_indices.len() && self.edit_cursor > 0 {
                let prev_char_start = char_indices[self.edit_cursor - 1].0;
                self.edit_buffer.remove(prev_char_start);
                self.edit_cursor -= 1;
            }
        }
    }

    /// Delete character at cursor (delete key)
    pub fn delete_char(&mut self) {
        if self.editing {
            let char_count = self.edit_buffer.chars().count();
            if self.edit_cursor < char_count {
                let byte_pos = self.edit_buffer
                    .char_indices()
                    .nth(self.edit_cursor)
                    .map(|(i, _)| i)
                    .unwrap_or(self.edit_buffer.len());
                self.edit_buffer.remove(byte_pos);
            }
        }
    }

    /// Move cursor left
    pub fn cursor_left(&mut self) {
        if self.editing && self.edit_cursor > 0 {
            self.edit_cursor -= 1;
        }
    }

    /// Move cursor right
    pub fn cursor_right(&mut self) {
        if self.editing {
            let char_count = self.edit_buffer.chars().count();
            if self.edit_cursor < char_count {
                self.edit_cursor += 1;
            }
        }
    }

    /// Move cursor to start
    pub fn cursor_home(&mut self) {
        if self.editing {
            self.edit_cursor = 0;
        }
    }

    /// Move cursor to end
    pub fn cursor_end(&mut self) {
        if self.editing {
            self.edit_cursor = self.edit_buffer.chars().count();
        }
    }

    /// Clear the edit buffer
    pub fn clear_edit(&mut self) {
        if self.editing {
            self.edit_buffer.clear();
            self.edit_cursor = 0;
        }
    }

    /// Delete word before cursor (Ctrl+W)
    pub fn delete_word(&mut self) {
        if self.editing && self.edit_cursor > 0 {
            let chars: Vec<char> = self.edit_buffer.chars().collect();
            let mut new_cursor = self.edit_cursor;

            // Skip trailing spaces
            while new_cursor > 0 && chars[new_cursor - 1].is_whitespace() {
                new_cursor -= 1;
            }

            // Delete word characters
            while new_cursor > 0 && !chars[new_cursor - 1].is_whitespace() {
                new_cursor -= 1;
            }

            // Remove the characters
            let start_byte = chars[..new_cursor]
                .iter()
                .map(|c| c.len_utf8())
                .sum::<usize>();
            let end_byte = chars[..self.edit_cursor]
                .iter()
                .map(|c| c.len_utf8())
                .sum::<usize>();

            self.edit_buffer = format!(
                "{}{}",
                &self.edit_buffer[..start_byte],
                &self.edit_buffer[end_byte..]
            );
            self.edit_cursor = new_cursor;
        }
    }

    /// Adjust scroll offset to keep cursor visible
    pub fn adjust_scroll(&mut self, visible_width: usize) {
        if visible_width == 0 {
            return;
        }
        let margin = 2.min(visible_width / 4);
        if self.edit_cursor < self.edit_scroll + margin {
            self.edit_scroll = self.edit_cursor.saturating_sub(margin);
        } else if self.edit_cursor >= self.edit_scroll + visible_width - margin {
            self.edit_scroll = self.edit_cursor.saturating_sub(visible_width - margin - 1);
        }
    }

    // ========================================================================
    // Field Manipulation
    // ========================================================================

    /// Toggle the selected toggle field
    pub fn toggle_selected(&mut self) {
        if let Some(field) = self.selected_field_mut() {
            field.kind.toggle_bool();
        }
    }

    /// Cycle the selected select field
    pub fn cycle_selected(&mut self) {
        if let Some(field) = self.selected_field_mut() {
            field.kind.cycle_next();
        }
    }

    /// Increment the selected number field
    pub fn increment_selected(&mut self) {
        if let Some(field) = self.selected_field_mut() {
            field.kind.increment();
        }
    }

    /// Decrement the selected number field
    pub fn decrement_selected(&mut self) {
        if let Some(field) = self.selected_field_mut() {
            field.kind.decrement();
        }
    }

    // ========================================================================
    // Scrolling
    // ========================================================================

    /// Scroll the selected scrollable field up
    pub fn scroll_up(&mut self, amount: usize) {
        if let Some(field) = self.selected_field_mut() {
            match &mut field.kind {
                FieldKind::ScrollableContent { scroll_offset, .. } => {
                    *scroll_offset = scroll_offset.saturating_sub(amount);
                }
                FieldKind::List { scroll_offset, .. } => {
                    *scroll_offset = scroll_offset.saturating_sub(amount);
                }
                _ => {}
            }
        }
    }

    /// Scroll the selected scrollable field down
    pub fn scroll_down(&mut self, amount: usize) {
        // Get actual content height if available (set during rendering)
        let actual_height = self.actual_content_height;
        if let Some(field) = self.selected_field_mut() {
            match &mut field.kind {
                FieldKind::ScrollableContent { lines, scroll_offset, visible_height } => {
                    // Use actual rendered height if available, otherwise fall back to visible_height
                    let effective_height = actual_height.unwrap_or(*visible_height);
                    let max_scroll = lines.len().saturating_sub(effective_height);
                    *scroll_offset = (*scroll_offset + amount).min(max_scroll);
                }
                FieldKind::List { items, scroll_offset, visible_height, .. } => {
                    let max_scroll = items.len().saturating_sub(*visible_height);
                    *scroll_offset = (*scroll_offset + amount).min(max_scroll);
                }
                _ => {}
            }
        }
    }

    /// Scroll to the beginning of the selected scrollable field
    pub fn scroll_to_top(&mut self) {
        if let Some(field) = self.selected_field_mut() {
            match &mut field.kind {
                FieldKind::ScrollableContent { scroll_offset, .. } => {
                    *scroll_offset = 0;
                }
                FieldKind::List { scroll_offset, .. } => {
                    *scroll_offset = 0;
                }
                _ => {}
            }
        }
    }

    /// Scroll to the end of the selected scrollable field
    pub fn scroll_to_bottom(&mut self) {
        if let Some(field) = self.selected_field_mut() {
            match &mut field.kind {
                FieldKind::ScrollableContent { lines, scroll_offset, visible_height } => {
                    *scroll_offset = lines.len().saturating_sub(*visible_height);
                }
                FieldKind::List { items, scroll_offset, visible_height, .. } => {
                    *scroll_offset = items.len().saturating_sub(*visible_height);
                }
                _ => {}
            }
        }
    }

    /// Move selection up in a list field
    pub fn list_select_up(&mut self) {
        // Find the first list field and update its selection
        for field in &mut self.definition.fields {
            if let FieldKind::List { selected_index, scroll_offset, .. } = &mut field.kind {
                if *selected_index > 0 {
                    *selected_index -= 1;
                    // Scroll to keep selection visible
                    if *selected_index < *scroll_offset {
                        *scroll_offset = *selected_index;
                    }
                }
                return;
            }
        }
    }

    /// Move selection down in a list field
    pub fn list_select_down(&mut self) {
        // Find the first list field and update its selection
        for field in &mut self.definition.fields {
            if let FieldKind::List { items, selected_index, scroll_offset, visible_height, .. } = &mut field.kind {
                if *selected_index + 1 < items.len() {
                    *selected_index += 1;
                    // Scroll to keep selection visible, but never scroll past last item
                    if *selected_index >= *scroll_offset + *visible_height {
                        let new_offset = selected_index.saturating_sub(*visible_height - 1);
                        // Limit scroll so we don't show empty space at bottom
                        let max_scroll = items.len().saturating_sub(*visible_height);
                        *scroll_offset = new_offset.min(max_scroll);
                    }
                }
                return;
            }
        }
    }

    /// Get the currently selected item in a list field
    pub fn get_selected_list_item(&self) -> Option<&ListItem> {
        // Find the first list field and get its selected item
        for field in &self.definition.fields {
            if let FieldKind::List { items, selected_index, .. } = &field.kind {
                return items.get(*selected_index);
            }
        }
        None
    }

    // ========================================================================
    // Custom State
    // ========================================================================

    /// Get custom state value
    pub fn get_custom(&self, key: &str) -> Option<&str> {
        self.custom.get(key).map(|s| s.as_str())
    }

    /// Set custom state value
    pub fn set_custom(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.custom.insert(key.into(), value.into());
    }
}

// ============================================================================
// Popup Manager
// ============================================================================

/// Manages multiple popups
pub struct PopupManager {
    /// Currently open popup (only one at a time)
    current: Option<PopupState>,
    /// Stack of popups for nested dialogs (e.g., confirm delete)
    stack: Vec<PopupState>,
}

impl PopupManager {
    pub fn new() -> Self {
        Self {
            current: None,
            stack: Vec::new(),
        }
    }

    /// Open a popup from a definition
    pub fn open(&mut self, definition: PopupDefinition) {
        let mut state = PopupState::new(definition);
        state.open();
        self.current = Some(state);
    }

    /// Push current popup to stack and open a new one (for nested dialogs)
    pub fn push(&mut self, definition: PopupDefinition) {
        if let Some(current) = self.current.take() {
            self.stack.push(current);
        }
        let mut state = PopupState::new(definition);
        state.open();
        self.current = Some(state);
    }

    /// Close current popup and pop from stack if available
    pub fn close(&mut self) {
        self.current = self.stack.pop();
    }

    /// Close all popups
    pub fn close_all(&mut self) {
        self.current = None;
        self.stack.clear();
    }

    /// Get current popup state
    pub fn current(&self) -> Option<&PopupState> {
        self.current.as_ref()
    }

    /// Get mutable current popup state
    pub fn current_mut(&mut self) -> Option<&mut PopupState> {
        self.current.as_mut()
    }

    /// Check if any popup is open
    pub fn is_open(&self) -> bool {
        self.current.is_some()
    }

    /// Check if a specific popup is open
    pub fn is_popup_open(&self, id: &PopupId) -> bool {
        self.current
            .as_ref()
            .map(|s| &s.definition.id == id)
            .unwrap_or(false)
    }
}

impl Default for PopupManager {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    const FIELD_NAME: FieldId = FieldId(1);
    const FIELD_EMAIL: FieldId = FieldId(2);
    const FIELD_ENABLED: FieldId = FieldId(3);
    const BTN_SAVE: ButtonId = ButtonId(1);
    const BTN_CANCEL: ButtonId = ButtonId(2);

    fn create_test_popup() -> PopupDefinition {
        PopupDefinition::new(PopupId("test"), "Test Popup")
            .with_field(Field::new(FIELD_NAME, "Name", FieldKind::text("")))
            .with_field(Field::new(FIELD_EMAIL, "Email", FieldKind::text("")))
            .with_field(Field::new(FIELD_ENABLED, "Enabled", FieldKind::toggle(true)))
            .with_button(Button::new(BTN_SAVE, "Save").primary().with_shortcut('S'))
            .with_button(Button::new(BTN_CANCEL, "Cancel").with_shortcut('C'))
    }

    #[test]
    fn test_popup_state_creation() {
        let def = create_test_popup();
        let state = PopupState::new(def);

        assert!(!state.visible);
        assert!(matches!(state.selected, ElementSelection::Field(FIELD_NAME)));
    }

    #[test]
    fn test_field_navigation() {
        let def = create_test_popup();
        let mut state = PopupState::new(def);
        state.open();

        assert!(matches!(state.selected, ElementSelection::Field(FIELD_NAME)));

        // Move to next field
        assert!(state.next_field());
        assert!(matches!(state.selected, ElementSelection::Field(FIELD_EMAIL)));

        // Move to next field
        assert!(state.next_field());
        assert!(matches!(state.selected, ElementSelection::Field(FIELD_ENABLED)));

        // Can't move past last field
        assert!(!state.next_field());
        assert!(matches!(state.selected, ElementSelection::Field(FIELD_ENABLED)));

        // Move back
        assert!(state.prev_field());
        assert!(matches!(state.selected, ElementSelection::Field(FIELD_EMAIL)));
    }

    #[test]
    fn test_button_navigation() {
        let def = create_test_popup();
        let mut state = PopupState::new(def);
        state.open();

        // Jump to buttons
        state.select_first_button();
        assert!(matches!(state.selected, ElementSelection::Button(BTN_SAVE)));

        // Cycle buttons
        state.next_button();
        assert!(matches!(state.selected, ElementSelection::Button(BTN_CANCEL)));

        state.next_button();
        assert!(matches!(state.selected, ElementSelection::Button(BTN_SAVE)));

        state.prev_button();
        assert!(matches!(state.selected, ElementSelection::Button(BTN_CANCEL)));
    }

    #[test]
    fn test_text_editing() {
        let def = create_test_popup();
        let mut state = PopupState::new(def);
        state.open();

        // Start editing
        state.start_edit();
        assert!(state.editing);
        assert_eq!(state.edit_buffer, "");
        assert_eq!(state.edit_cursor, 0);

        // Insert characters
        state.insert_char('H');
        state.insert_char('e');
        state.insert_char('l');
        state.insert_char('l');
        state.insert_char('o');
        assert_eq!(state.edit_buffer, "Hello");
        assert_eq!(state.edit_cursor, 5);

        // Move cursor
        state.cursor_left();
        assert_eq!(state.edit_cursor, 4);

        state.cursor_home();
        assert_eq!(state.edit_cursor, 0);

        state.cursor_end();
        assert_eq!(state.edit_cursor, 5);

        // Commit edit
        state.commit_edit();
        assert!(!state.editing);
        assert_eq!(state.get_text(FIELD_NAME), Some("Hello"));
    }

    #[test]
    fn test_toggle_field() {
        let def = create_test_popup();
        let mut state = PopupState::new(def);
        state.open();

        // Navigate to toggle field
        state.select_field(FIELD_ENABLED);
        assert_eq!(state.get_bool(FIELD_ENABLED), Some(true));

        // Toggle
        state.toggle_selected();
        assert_eq!(state.get_bool(FIELD_ENABLED), Some(false));

        state.toggle_selected();
        assert_eq!(state.get_bool(FIELD_ENABLED), Some(true));
    }

    #[test]
    fn test_select_field() {
        let options = vec![
            SelectOption::simple("utf8"),
            SelectOption::simple("latin1"),
            SelectOption::simple("fansi"),
        ];
        let def = PopupDefinition::new(PopupId("test"), "Test")
            .with_field(Field::new(FieldId(1), "Encoding", FieldKind::select(options, 0)));
        let mut state = PopupState::new(def);
        state.open();

        assert_eq!(state.get_selected(FieldId(1)), Some("utf8"));

        state.cycle_selected();
        assert_eq!(state.get_selected(FieldId(1)), Some("latin1"));

        state.cycle_selected();
        assert_eq!(state.get_selected(FieldId(1)), Some("fansi"));

        state.cycle_selected();
        assert_eq!(state.get_selected(FieldId(1)), Some("utf8"));
    }

    #[test]
    fn test_number_field() {
        let def = PopupDefinition::new(PopupId("test"), "Test")
            .with_field(Field::new(FieldId(1), "Height", FieldKind::number_range(5, 1, 10)));
        let mut state = PopupState::new(def);
        state.open();

        assert_eq!(state.get_number(FieldId(1)), Some(5));

        state.increment_selected();
        assert_eq!(state.get_number(FieldId(1)), Some(6));

        // Test clamping at max
        for _ in 0..10 {
            state.increment_selected();
        }
        assert_eq!(state.get_number(FieldId(1)), Some(10));

        // Test clamping at min
        for _ in 0..20 {
            state.decrement_selected();
        }
        assert_eq!(state.get_number(FieldId(1)), Some(1));
    }

    #[test]
    fn test_popup_manager() {
        let mut manager = PopupManager::new();
        assert!(!manager.is_open());

        // Open popup
        manager.open(create_test_popup());
        assert!(manager.is_open());
        assert!(manager.is_popup_open(&PopupId("test")));

        // Close popup
        manager.close();
        assert!(!manager.is_open());
    }

    #[test]
    fn test_nested_popups() {
        let mut manager = PopupManager::new();

        // Open first popup
        manager.open(PopupDefinition::new(PopupId("first"), "First"));
        assert!(manager.is_popup_open(&PopupId("first")));

        // Push second popup
        manager.push(PopupDefinition::new(PopupId("second"), "Second"));
        assert!(manager.is_popup_open(&PopupId("second")));

        // Close second, first should be restored
        manager.close();
        assert!(manager.is_popup_open(&PopupId("first")));

        // Close first
        manager.close();
        assert!(!manager.is_open());
    }

    #[test]
    fn test_scrollable_content() {
        let lines: Vec<String> = (0..50).map(|i| format!("Line {}", i)).collect();
        let def = PopupDefinition::new(PopupId("test"), "Test")
            .with_field(Field::new(
                FieldId(1),
                "",
                FieldKind::scrollable_content(lines, 10),
            ));
        let mut state = PopupState::new(def);
        state.open();

        // Check initial state
        if let Some(field) = state.field(FieldId(1)) {
            if let FieldKind::ScrollableContent { scroll_offset, .. } = &field.kind {
                assert_eq!(*scroll_offset, 0);
            }
        }

        // Scroll down
        state.scroll_down(5);
        if let Some(field) = state.field(FieldId(1)) {
            if let FieldKind::ScrollableContent { scroll_offset, .. } = &field.kind {
                assert_eq!(*scroll_offset, 5);
            }
        }

        // Scroll up
        state.scroll_up(3);
        if let Some(field) = state.field(FieldId(1)) {
            if let FieldKind::ScrollableContent { scroll_offset, .. } = &field.kind {
                assert_eq!(*scroll_offset, 2);
            }
        }

        // Scroll to bottom
        state.scroll_to_bottom();
        if let Some(field) = state.field(FieldId(1)) {
            if let FieldKind::ScrollableContent { scroll_offset, .. } = &field.kind {
                assert_eq!(*scroll_offset, 40); // 50 lines - 10 visible = 40
            }
        }

        // Scroll to top
        state.scroll_to_top();
        if let Some(field) = state.field(FieldId(1)) {
            if let FieldKind::ScrollableContent { scroll_offset, .. } = &field.kind {
                assert_eq!(*scroll_offset, 0);
            }
        }
    }

    #[test]
    fn test_list_selection() {
        let items: Vec<ListItem> = (0..20)
            .map(|i| ListItem {
                id: format!("item_{}", i),
                columns: vec![format!("Item {}", i)],
                style: ListItemStyle::default(),
            })
            .collect();
        let def = PopupDefinition::new(PopupId("test"), "Test")
            .with_field(Field::new(
                FieldId(1),
                "",
                FieldKind::list(items, 5),
            ));
        let mut state = PopupState::new(def);
        state.open();

        // Check initial selection
        assert_eq!(state.get_selected_list_item().map(|i| i.id.as_str()), Some("item_0"));

        // Move down
        state.list_select_down();
        assert_eq!(state.get_selected_list_item().map(|i| i.id.as_str()), Some("item_1"));

        // Move up
        state.list_select_up();
        assert_eq!(state.get_selected_list_item().map(|i| i.id.as_str()), Some("item_0"));

        // Try to move up past beginning
        state.list_select_up();
        assert_eq!(state.get_selected_list_item().map(|i| i.id.as_str()), Some("item_0"));
    }
}
