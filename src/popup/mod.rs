//! Unified popup/window management system
//!
//! This module provides a single data model for popups that can be rendered
//! by console (ratatui) and web (JavaScript) interfaces.

pub mod console_renderer;
pub mod definitions;

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

/// Common button ID for the help button used across all popups
pub const POPUP_BTN_HELP: ButtonId = ButtonId(99);

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
    /// Static validation error/warning message (read-only, no "Label: " prefix,
    /// drawn in the theme's error color). Used for e.g. the Web Settings popup's
    /// Save-blocking validation line — see `popup::definitions::web::validate_web_settings`.
    ErrorText { text: String },
    /// Visual separator line
    Separator,
    /// Multi-line text editor with scrolling viewport
    MultilineText {
        value: String,
        visible_lines: usize,
        scroll_offset: usize,
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
    /// Vertically-scrolling list of editable text items.
    /// The selected row is edited in-place using the shared PopupState edit_buffer /
    /// edit_cursor, with horizontal cursor-follow scroll identical to Text fields.
    /// Up/Down navigates rows (committing the current edit); navigating past the last
    /// non-empty row appends a new empty row.
    EditableList {
        items: Vec<String>,
        selected_index: usize,
        scroll_offset: usize,
        visible_height: usize,
    },
    /// Horizontal tab strip (see `console_renderer::render_tabs_field`). Two
    /// rendered rows: the labels, then the active tab's `═` underline.
    /// Windows itself against the popup's available width
    /// (`console_renderer::compute_tab_window`) rather than ever wrapping or
    /// scrolling vertically. `scroll_offset` caches the last-computed
    /// window's start column-index; it is a pure function's cached *output*,
    /// not an input the windowing decision reads back, so nothing in this
    /// file updates it — the renderer does, purely so an external reader
    /// (a future `/dump`, say) can see the window actually drawn.
    Tabs {
        labels: Vec<String>,
        selected_index: usize,
        scroll_offset: usize,
    },
    /// Grid of fixed-stride cells with a 2-D cursor (see
    /// `console_renderer::render_grid_field`). Reuses `ListItem`: `columns`
    /// (of the `ListItem`) = `[display glyph, name, keywords]`, `id` = the
    /// text to insert on Enter. `columns` (of this variant) is the grid's
    /// column count; every cell renders at a fixed stride regardless of the
    /// glyph's byte length — see `console_renderer::CELL_STRIDE` for why
    /// (multi-byte, double-width emoji break the byte/char-counting
    /// `render_list_field` uses for ordinary lists).
    Grid {
        cells: Vec<ListItem>,
        selected_index: usize,
        scroll_offset: usize,
        columns: usize,
        visible_rows: usize,
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

    /// Create a validation error/warning text field (see `FieldKind::ErrorText`)
    pub fn error_text(text: impl Into<String>) -> Self {
        Self::ErrorText { text: text.into() }
    }

    /// Create a separator
    pub fn separator() -> Self {
        Self::Separator
    }

    /// Create a multiline text field with a visible viewport height
    pub fn multiline(value: impl Into<String>, visible_lines: usize) -> Self {
        Self::MultilineText {
            value: value.into(),
            visible_lines,
            scroll_offset: 0,
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

    /// Create an editable list field (in-place row editing, vertical scroll)
    pub fn editable_list(items: Vec<String>, visible_height: usize) -> Self {
        Self::EditableList {
            items,
            selected_index: 0,
            scroll_offset: 0,
            visible_height,
        }
    }

    /// Create a horizontal tab strip field
    pub fn tabs(labels: Vec<String>, selected_index: usize) -> Self {
        Self::Tabs {
            labels,
            selected_index,
            scroll_offset: 0,
        }
    }

    /// Create a grid field with the given number of columns and viewport height
    pub fn grid(cells: Vec<ListItem>, columns: usize, visible_rows: usize) -> Self {
        Self::Grid {
            cells,
            selected_index: 0,
            scroll_offset: 0,
            columns,
            visible_rows,
        }
    }

    /// Get the string value for text-like fields.
    /// For EditableList, returns the currently selected item's text.
    /// Also readable (but never editable — see `is_text`) for `ErrorText`, so
    /// callers can inspect the current validation message the same way they'd
    /// read any other field.
    pub fn get_text(&self) -> Option<&str> {
        match self {
            Self::Text { value, .. } => Some(value),
            Self::MultilineText { value, .. } => Some(value),
            Self::EditableList { items, selected_index, .. } => {
                items.get(*selected_index).map(|s| s.as_str())
            }
            Self::ErrorText { text } => Some(text),
            _ => None,
        }
    }

    /// Check if this is a text-like field (supports text editing)
    pub fn is_text(&self) -> bool {
        matches!(self, Self::Text { .. } | Self::MultilineText { .. } | Self::EditableList { .. })
    }

    /// Set the string value for text-like fields.
    /// For EditableList, sets the currently selected item's text.
    pub fn set_text(&mut self, new_value: String) {
        match self {
            Self::Text { value, .. } => *value = new_value,
            Self::MultilineText { value, .. } => *value = new_value,
            Self::EditableList { items, selected_index, .. } => {
                if let Some(item) = items.get_mut(*selected_index) {
                    *item = new_value;
                }
            }
            _ => {}
        }
    }

    /// Get all items from an EditableList field
    pub fn get_items(&self) -> Option<&[String]> {
        if let Self::EditableList { items, .. } = self {
            Some(items)
        } else {
            None
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
        !matches!(self, Self::Label { .. } | Self::Separator | Self::ErrorText { .. })
    }

    /// Check if this is a text-editable field
    pub fn is_text_editable(&self) -> bool {
        matches!(self, Self::Text { .. } | Self::MultilineText { .. } | Self::EditableList { .. })
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
    /// Tab order index (lower = earlier in tab cycle). None = use definition order after indexed elements.
    pub tab_index: Option<u32>,
    /// Whether this is a filter/search field. Search fields are always ordered
    /// first in the navigation cycle (see `PopupDefinition::ordered_elements`).
    pub search: bool,
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
            tab_index: None,
            search: false,
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
            tab_index: None,
            search: false,
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

    /// Set the tab order index for this field
    pub fn with_tab_index(mut self, index: u32) -> Self {
        self.tab_index = Some(index);
        self
    }

    /// Mark this as a filter/search field. Search fields are ordered first
    /// in the navigation cycle (order of moving: search -> inputs -> right
    /// buttons -> left buttons).
    pub fn search(mut self) -> Self {
        self.search = true;
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
    pub left_align: bool,
    /// Tab order index (lower = earlier in tab cycle). None = use definition order after indexed elements.
    pub tab_index: Option<u32>,
}

impl Button {
    pub fn new(id: ButtonId, label: impl Into<String>) -> Self {
        Self {
            id,
            label: label.into(),
            shortcut: None,
            style: ButtonStyle::Secondary,
            enabled: true,
            left_align: false,
            tab_index: None,
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

    pub fn left_align(mut self) -> Self {
        self.left_align = true;
        self
    }

    pub fn with_tab_index(mut self, index: u32) -> Self {
        self.tab_index = Some(index);
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
    /// Whether Tab should only cycle through buttons (skip fields)
    pub tab_buttons_only: bool,
    /// Anchor popup to bottom (just above separator bar) at anchor_x position
    pub anchor_bottom_left: bool,
    /// X position for anchor_bottom_left (0 = left edge)
    pub anchor_x: u16,
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
            tab_buttons_only: false,
            anchor_bottom_left: false,
            anchor_x: 0,
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
    /// Help text lines shown when the user presses the ? button
    pub help_lines: Vec<String>,
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
            help_lines: Vec::new(),
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

    pub fn with_button_if(self, condition: bool, button: Button) -> Self {
        if condition { self.with_button(button) } else { self }
    }

    pub fn with_layout(mut self, layout: PopupLayout) -> Self {
        self.layout = layout;
        self
    }

    /// Add help text and a ? button to this popup
    pub fn with_help(mut self, lines: Vec<String>) -> Self {
        self.help_lines = lines;
        self.buttons.insert(0, Button::new(POPUP_BTN_HELP, "?").left_align().with_shortcut('?'));
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

    /// Build the canonical "order of moving" for this popup:
    /// 1. Filter/search fields
    /// 2. Other focusable fields (in definition/tab_index order)
    /// 3. Right-aligned enabled buttons
    /// 4. Left-aligned enabled buttons
    ///
    /// Within each group, elements with an explicit `tab_index` are sorted by
    /// that index and come first; the rest follow in definition order. This
    /// generalizes the sort used by the old `cycle_field_buttons` to cover
    /// every focusable field kind (not just text-editable ones), since Up/Down
    /// navigation must be able to reach Toggle/Select/Number/List fields too.
    ///
    /// Returns (is_button, id) pairs in navigation order.
    pub fn ordered_elements(&self) -> Vec<(bool, u32)> {
        fn sort_group(group: &mut [(bool, u32, u32)]) {
            group.sort_by(|a, b| match (a.0, b.0) {
                (true, true) => a.1.cmp(&b.1),
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                (false, false) => a.1.cmp(&b.1),
            });
        }

        let mut search_fields: Vec<(bool, u32, u32)> = Vec::new();
        let mut other_fields: Vec<(bool, u32, u32)> = Vec::new();
        let mut right_buttons: Vec<(bool, u32, u32)> = Vec::new();
        let mut left_buttons: Vec<(bool, u32, u32)> = Vec::new();

        for (def_idx, field) in self.fields.iter().enumerate() {
            if !field.is_focusable() {
                continue;
            }
            let (has_idx, idx) = match field.tab_index {
                Some(i) => (true, i),
                None => (false, def_idx as u32),
            };
            if field.search {
                search_fields.push((has_idx, idx, field.id.0));
            } else {
                other_fields.push((has_idx, idx, field.id.0));
            }
        }

        for (def_idx, button) in self.buttons.iter().enumerate() {
            if !button.enabled {
                continue;
            }
            let (has_idx, idx) = match button.tab_index {
                Some(i) => (true, i),
                None => (false, def_idx as u32),
            };
            if button.left_align {
                left_buttons.push((has_idx, idx, button.id.0));
            } else {
                right_buttons.push((has_idx, idx, button.id.0));
            }
        }

        sort_group(&mut search_fields);
        sort_group(&mut other_fields);
        // Buttons are NOT sorted by tab_index: they must navigate in the same
        // left-to-right order the console renderer draws them in (definition
        // order), or Tab/Up/Down would visit a button group in a different
        // order than what's on screen. See render_buttons() in
        // console_renderer.rs, which lays out each group in definition order.

        let mut result = Vec::with_capacity(
            search_fields.len() + other_fields.len() + right_buttons.len() + left_buttons.len(),
        );
        result.extend(search_fields.into_iter().map(|(_, _, id)| (false, id)));
        result.extend(other_fields.into_iter().map(|(_, _, id)| (false, id)));
        result.extend(right_buttons.into_iter().map(|(_, _, id)| (true, id)));
        result.extend(left_buttons.into_iter().map(|(_, _, id)| (true, id)));
        result
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
    /// When the error was set (for auto-clear)
    pub error_at: Option<std::time::Instant>,
    /// Vertical scroll offset into the popup's top-level field list, in whole-field units —
    /// used only when the popup has more scalar fields (Toggle/Select/Number/Text/...) than
    /// fit the visible area (see `render_popup_content` in console_renderer.rs, which both
    /// clamps this to keep the selected field visible and renders the matching scrollbar).
    /// Distinct from a `FieldKind::List`/`ScrollableContent`/`EditableList` field's own
    /// internal `scroll_offset`, which lives on the field itself, not here.
    pub scroll_offset: usize,
    /// Whether the top-level field list currently needs `scroll_offset` (i.e. there are
    /// more fields than fit the visible area) — set fresh every frame by
    /// `render_popup_content` in console_renderer.rs. When true, Up/Down navigation and
    /// the mouse wheel are confined to the field list (see `select_field_only`) instead of
    /// cycling into the buttons, so the buttons stay reachable only via Tab/Shift+Tab.
    pub field_list_needs_scroll: bool,
    /// Custom state for complex popups (e.g., filter text)
    pub custom: HashMap<String, String>,
    /// Actual rendered content height (set during rendering, used for scroll calculations)
    pub actual_content_height: Option<usize>,
    /// Hit areas for mouse click detection (populated during console rendering)
    pub hit_areas: Vec<(ratatui::layout::Rect, ElementSelection)>,
    /// Mouse text highlight state
    pub highlight: Option<PopupHighlight>,
    /// Content areas for mouse-to-line mapping (populated during console rendering)
    pub content_areas: Vec<ContentArea>,
}

/// Mouse text highlight state for popup content
#[derive(Debug, Clone)]
pub struct PopupHighlight {
    /// Which field contains the highlight
    pub field_id: FieldId,
    /// Start line index (content-relative, inclusive)
    pub start_line: usize,
    /// End line index (content-relative, inclusive)
    pub end_line: usize,
    /// Whether the user is currently dragging
    pub dragging: bool,
}

/// Describes a content area for mouse-to-line mapping
#[derive(Debug, Clone)]
pub struct ContentArea {
    /// Screen area where content lines are rendered (excludes headers/scrollbar)
    pub area: ratatui::layout::Rect,
    /// Which field this content area belongs to
    pub field_id: FieldId,
    /// Current scroll offset of the content
    pub scroll_offset: usize,
    /// Total number of content lines
    pub total_lines: usize,
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
            error_at: None,
            scroll_offset: 0,
            field_list_needs_scroll: false,
            custom: HashMap::new(),
            actual_content_height: None,
            hit_areas: Vec::new(),
            highlight: None,
            content_areas: Vec::new(),
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

    /// Find the position of the currently selected element within an
    /// `ordered_elements()` list.
    fn position_in(&self, elements: &[(bool, u32)]) -> Option<usize> {
        match &self.selected {
            ElementSelection::Field(id) => {
                elements.iter().position(|(is_btn, elem_id)| !is_btn && *elem_id == id.0)
            }
            ElementSelection::Button(id) => {
                elements.iter().position(|(is_btn, elem_id)| *is_btn && *elem_id == id.0)
            }
            ElementSelection::None => None,
        }
    }

    fn select_element(&mut self, (is_button, id): (bool, u32)) {
        self.selected = if is_button {
            ElementSelection::Button(ButtonId(id))
        } else {
            ElementSelection::Field(FieldId(id))
        };
    }

    /// Tab key: from a field, jump straight to the first button; from a
    /// button, move to the next button; from the last button, wrap around to
    /// the first field (first search field, else first input field). Popups
    /// with no buttons simply cycle through fields; popups with no fields
    /// simply cycle through buttons.
    /// Returns true if selection changed.
    pub fn cycle_field_buttons(&mut self) -> bool {
        let elements = self.definition.ordered_elements();
        if elements.is_empty() {
            return false;
        }

        let current_pos = self.position_in(&elements);
        let next_pos = match current_pos {
            Some(pos) if !elements[pos].0 => {
                // On a field: jump to the first button, if any; otherwise
                // just advance to the next field (no buttons to tab to).
                elements.iter().position(|(is_btn, _)| *is_btn).unwrap_or((pos + 1) % elements.len())
            }
            Some(pos) => {
                // On a button: advance to the next button, or wrap to the
                // first field (or first button, if there are no fields).
                let next = pos + 1;
                if next < elements.len() && elements[next].0 {
                    next
                } else {
                    elements.iter().position(|(is_btn, _)| !is_btn).unwrap_or(0)
                }
            }
            None => 0,
        };

        self.select_element(elements[next_pos]);
        true
    }

    /// Shift+Tab: the exact reverse of `cycle_field_buttons`.
    /// Returns true if selection changed.
    pub fn cycle_field_buttons_rev(&mut self) -> bool {
        let elements = self.definition.ordered_elements();
        if elements.is_empty() {
            return false;
        }

        let current_pos = self.position_in(&elements);
        let prev_pos = match current_pos {
            Some(pos) if elements[pos].0 => {
                // On a button: previous button, or wrap to the last field
                // (or last button, if there are no fields).
                if pos > 0 && elements[pos - 1].0 {
                    pos - 1
                } else {
                    elements.iter().rposition(|(is_btn, _)| !is_btn).unwrap_or(elements.len() - 1)
                }
            }
            Some(pos) => {
                // On a field: previous field, or wrap to the last button
                // (or last field, if there are no buttons).
                if pos > 0 && !elements[pos - 1].0 {
                    pos - 1
                } else {
                    elements.iter().rposition(|(is_btn, _)| *is_btn).unwrap_or(elements.len() - 1)
                }
            }
            None => elements.len() - 1,
        };

        self.select_element(elements[prev_pos]);
        true
    }

    /// Move selection to the next/prev FIELD only, clamped at both ends — never onto a
    /// button, and never wrapping past the first/last field either. Used instead of the
    /// button-inclusive cycle when `field_list_needs_scroll` is set (popups with more
    /// scalar fields than fit the screen, e.g. `/setup`, `/world -e`), so Up/Down/the
    /// mouse wheel stay confined to the field list and only Tab/Shift+Tab reach the
    /// buttons — matching how a `FieldKind::List` field already keeps arrow/wheel
    /// navigation confined to itself.
    fn select_field_only(&mut self, forward: bool) {
        let elements = self.definition.ordered_elements();
        let field_positions: Vec<usize> = elements.iter()
            .enumerate()
            .filter(|(_, (is_btn, _))| !is_btn)
            .map(|(i, _)| i)
            .collect();
        if field_positions.is_empty() {
            return;
        }
        let current_field_idx = self.position_in(&elements)
            .and_then(|pos| field_positions.iter().position(|&fp| fp == pos));
        let next_field_idx = match current_field_idx {
            Some(idx) if forward => (idx + 1).min(field_positions.len() - 1),
            Some(idx) => idx.saturating_sub(1),
            None => 0,
        };
        self.select_element(elements[field_positions[next_field_idx]]);
    }

    /// Down key: move forward through the "order of moving" (search fields,
    /// then other fields, then right buttons, then left buttons), wrapping
    /// around. If the currently selected field is a List or ScrollableContent,
    /// this instead scrolls/moves the selection *within* that field, matching
    /// "list/menu/help popups keep arrow-scrolling inside their list." Same idea when
    /// `field_list_needs_scroll` is set — stays within the field list, never escaping
    /// to the buttons (see `select_field_only`).
    pub fn next_item(&mut self) {
        if let Some(field) = self.selected_field() {
            match &field.kind {
                FieldKind::List { .. } => { self.list_select_down(); return; }
                FieldKind::ScrollableContent { .. } => { self.scroll_down(1); return; }
                FieldKind::Grid { .. } => { self.grid_move(0, 1); return; }
                _ => {}
            }
        }
        if self.field_list_needs_scroll && matches!(self.selected, ElementSelection::Field(_)) {
            self.select_field_only(true);
            return;
        }
        let elements = self.definition.ordered_elements();
        if elements.is_empty() {
            return;
        }
        let next_pos = match self.position_in(&elements) {
            Some(pos) => (pos + 1) % elements.len(),
            None => 0,
        };
        self.select_element(elements[next_pos]);
    }

    /// Up key: the exact reverse of `next_item`.
    pub fn prev_item(&mut self) {
        if let Some(field) = self.selected_field() {
            match &field.kind {
                FieldKind::List { .. } => { self.list_select_up(); return; }
                FieldKind::ScrollableContent { .. } => { self.scroll_up(1); return; }
                FieldKind::Grid { .. } => { self.grid_move(0, -1); return; }
                _ => {}
            }
        }
        if self.field_list_needs_scroll && matches!(self.selected, ElementSelection::Field(_)) {
            self.select_field_only(false);
            return;
        }
        let elements = self.definition.ordered_elements();
        if elements.is_empty() {
            return;
        }
        let prev_pos = match self.position_in(&elements) {
            Some(pos) => if pos == 0 { elements.len() - 1 } else { pos - 1 },
            None => elements.len() - 1,
        };
        self.select_element(elements[prev_pos]);
    }

    /// Find an enabled button whose shortcut letter matches (case-insensitive).
    /// Used to dispatch a bare keypress (when not editing) to invoke that
    /// button, per the "button hotkey letter" navigation rule.
    pub fn find_button_by_shortcut(&self, key: char) -> Option<ButtonId> {
        let key_lower = key.to_ascii_lowercase();
        self.definition.buttons.iter()
            .find(|b| b.enabled && b.shortcut.map(|s| s.to_ascii_lowercase()) == Some(key_lower))
            .map(|b| b.id)
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

    /// Insert a whole string at the cursor (e.g. a paste) in one pass, rather than
    /// looping `insert_char` per character. `edit_cursor` is a character index here,
    /// same convention as `insert_char`.
    pub fn insert_str(&mut self, s: &str) {
        if self.editing && !s.is_empty() {
            let byte_pos = self.edit_buffer
                .char_indices()
                .nth(self.edit_cursor)
                .map(|(i, _)| i)
                .unwrap_or(self.edit_buffer.len());
            self.edit_buffer.insert_str(byte_pos, s);
            self.edit_cursor += s.chars().count();
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

    /// Move cursor up one line in multiline text
    pub fn cursor_up(&mut self) {
        if !self.editing {
            return;
        }

        let chars: Vec<char> = self.edit_buffer.chars().collect();

        // Find current line start and column
        let mut current_line_start = 0;
        let mut current_col = self.edit_cursor;
        for (i, ch) in chars.iter().enumerate() {
            if i >= self.edit_cursor {
                break;
            }
            if *ch == '\n' {
                current_line_start = i + 1;
                current_col = self.edit_cursor - (i + 1);
            }
        }

        // If we're on the first line, can't go up
        if current_line_start == 0 {
            return;
        }

        // Find previous line start
        let mut prev_line_start = 0;
        for i in (0..current_line_start - 1).rev() {
            if chars[i] == '\n' {
                prev_line_start = i + 1;
                break;
            }
        }

        // Calculate previous line length
        let prev_line_len = current_line_start - 1 - prev_line_start;

        // Move to same column on previous line, or end of line if shorter
        self.edit_cursor = prev_line_start + current_col.min(prev_line_len);
    }

    /// Move cursor down one line in multiline text
    pub fn cursor_down(&mut self) {
        if !self.editing {
            return;
        }

        let chars: Vec<char> = self.edit_buffer.chars().collect();
        let total_len = chars.len();

        // Find current column position on current line
        let mut current_col = self.edit_cursor;
        for (i, ch) in chars.iter().enumerate() {
            if i >= self.edit_cursor {
                break;
            }
            if *ch == '\n' {
                current_col = self.edit_cursor - (i + 1);
            }
        }

        // Find next line start (after the newline following current position)
        let next_line_start = match chars[self.edit_cursor..total_len].iter().position(|&c| c == '\n') {
            Some(offset) => self.edit_cursor + offset + 1,
            None => return, // No next line
        };

        // Find next line length
        let next_line_len = chars[next_line_start..total_len].iter().take_while(|&&c| c != '\n').count();

        // Move to same column on next line, or end of line if shorter
        self.edit_cursor = next_line_start + current_col.min(next_line_len);
    }

    /// Insert a newline at cursor position
    pub fn insert_newline(&mut self) {
        self.insert_char('\n');
    }

    /// Get cursor display line number (0-indexed) accounting for line wrapping
    /// wrap_width is the character width for wrapping (typically the field width)
    pub fn get_cursor_display_line(&self, wrap_width: usize) -> usize {
        if wrap_width == 0 {
            return 0;
        }

        let mut display_line = 0;
        let mut char_offset = 0;
        let text_lines: Vec<&str> = self.edit_buffer.split('\n').collect();

        for (line_idx, text_line) in text_lines.iter().enumerate() {
            let line_len = text_line.chars().count();

            if line_len == 0 {
                // Empty line takes one display line
                if self.edit_cursor == char_offset {
                    return display_line;
                }
                display_line += 1;
            } else {
                // Wrap line into chunks of wrap_width
                let mut pos = 0;
                while pos < line_len {
                    let end = (pos + wrap_width).min(line_len);
                    let chunk_start = char_offset + pos;
                    let chunk_end = char_offset + end;

                    // Check if cursor is in this chunk
                    if self.edit_cursor >= chunk_start && self.edit_cursor <= chunk_end {
                        return display_line;
                    }

                    display_line += 1;
                    pos = end;
                }
            }

            // Account for the newline character (except for the last line)
            char_offset += line_len;
            if line_idx < text_lines.len() - 1 {
                char_offset += 1; // newline
            }
        }

        display_line.saturating_sub(1)
    }

    /// Ensure cursor is visible in multiline text field by adjusting scroll_offset
    /// Uses wrapping to calculate the display line
    pub fn ensure_multiline_cursor_visible(&mut self) {
        // Use a very conservative (small) wrap width to ensure scrolling happens
        // It's better to scroll more frequently than to have the cursor go off-screen
        // The actual rendering may use a wider wrap, but this ensures safety
        // Using 30 as a conservative minimum that works on most terminals
        let wrap_width = 30;
        let cursor_display_line = self.get_cursor_display_line(wrap_width);

        // Find the selected field and update its scroll_offset
        if let ElementSelection::Field(field_id) = &self.selected {
            for field in &mut self.definition.fields {
                if field.id == *field_id {
                    if let FieldKind::MultilineText { visible_lines, scroll_offset, .. } = &mut field.kind {
                        // Adjust scroll to keep cursor visible
                        if cursor_display_line < *scroll_offset {
                            *scroll_offset = cursor_display_line;
                        } else if cursor_display_line >= *scroll_offset + *visible_lines {
                            *scroll_offset = cursor_display_line - *visible_lines + 1;
                        }
                    }
                    break;
                }
            }
        }
    }

    /// Ensure cursor is visible with a specific wrap width
    pub fn ensure_multiline_cursor_visible_with_width(&mut self, wrap_width: usize) {
        let cursor_display_line = self.get_cursor_display_line(wrap_width);

        if let ElementSelection::Field(field_id) = &self.selected {
            for field in &mut self.definition.fields {
                if field.id == *field_id {
                    if let FieldKind::MultilineText { visible_lines, scroll_offset, .. } = &mut field.kind {
                        // Adjust scroll to keep cursor visible
                        if cursor_display_line < *scroll_offset {
                            *scroll_offset = cursor_display_line;
                        } else if cursor_display_line >= *scroll_offset + *visible_lines {
                            *scroll_offset = cursor_display_line - *visible_lines + 1;
                        }
                    }
                    break;
                }
            }
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

    /// Scroll the first scrollable content field up (regardless of selection)
    pub fn scroll_content_up(&mut self, amount: usize) {
        for field in &mut self.definition.fields {
            if let FieldKind::ScrollableContent { ref mut scroll_offset, .. } = field.kind {
                *scroll_offset = scroll_offset.saturating_sub(amount);
                return;
            }
        }
    }

    /// Scroll the first scrollable content field down (regardless of selection)
    pub fn scroll_content_down(&mut self, amount: usize) {
        let actual_height = self.actual_content_height;
        for field in &mut self.definition.fields {
            if let FieldKind::ScrollableContent { ref lines, ref mut scroll_offset, ref visible_height } = field.kind {
                let effective_height = actual_height.unwrap_or(*visible_height);
                let max_scroll = lines.len().saturating_sub(effective_height);
                *scroll_offset = (*scroll_offset + amount).min(max_scroll);
                return;
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

    /// Move selection up in the EditableList field with given id.
    /// Commits any in-progress edit before moving.
    /// Returns true if the row actually changed (false if already at first row).
    pub fn editable_list_select_up(&mut self, id: FieldId) -> bool {
        self.commit_edit();
        if let Some(field) = self.definition.get_field_mut(id) {
            if let FieldKind::EditableList { selected_index, scroll_offset, .. } = &mut field.kind {
                if *selected_index > 0 {
                    *selected_index -= 1;
                    if *selected_index < *scroll_offset {
                        *scroll_offset = *selected_index;
                    }
                    return true;
                }
            }
        }
        false
    }

    /// Move selection down in the EditableList field with given id.
    /// Commits any in-progress edit before moving.
    /// If at the last non-empty row, appends an empty row and moves into it.
    /// Returns true if the row changed.
    pub fn editable_list_select_down(&mut self, id: FieldId) -> bool {
        self.commit_edit();
        if let Some(field) = self.definition.get_field_mut(id) {
            if let FieldKind::EditableList { items, selected_index, scroll_offset, visible_height } = &mut field.kind {
                let next = *selected_index + 1;
                // If moving past the last item, append an empty row (but only one trailing empty)
                if next >= items.len() {
                    // Only append if the current last item is non-empty
                    let last_non_empty = items.last().map(|s| !s.is_empty()).unwrap_or(false);
                    if last_non_empty {
                        items.push(String::new());
                    } else {
                        return false; // Already have a trailing empty row
                    }
                }
                *selected_index = next;
                if *selected_index >= *scroll_offset + *visible_height {
                    let new_offset = selected_index.saturating_sub(*visible_height - 1);
                    let max_scroll = items.len().saturating_sub(*visible_height);
                    *scroll_offset = new_offset.min(max_scroll);
                }
                return true;
            }
        }
        false
    }

    /// Get all items from the EditableList field with given id.
    /// Flushes any in-progress edit first.
    pub fn get_editable_list_items(&self, id: FieldId) -> Vec<String> {
        if let Some(field) = self.definition.get_field(id) {
            if let FieldKind::EditableList { items, .. } = &field.kind {
                return items.clone();
            }
        }
        Vec::new()
    }

    /// Handle mouse scroll: moves list selection or scrolls content
    pub fn mouse_scroll_up(&mut self) {
        for field in &self.definition.fields {
            if matches!(&field.kind, FieldKind::List { .. }) {
                self.list_select_up();
                return;
            }
        }
        if self.definition.fields.iter().any(|f| matches!(&f.kind, FieldKind::ScrollableContent { .. })) {
            // Scroll the first scrollable content field (works even when button is selected)
            self.scroll_any_content_up(1);
            return;
        }
        // No List/ScrollableContent field — for popups with a scrollable top-level field
        // list (e.g. /setup, /world -e), move the selection up through the fields, same as
        // pressing Up (see select_field_only).
        if self.field_list_needs_scroll {
            self.select_field_only(false);
        }
    }

    /// Handle mouse scroll down: moves list selection or scrolls content
    pub fn mouse_scroll_down(&mut self) {
        for field in &self.definition.fields {
            if matches!(&field.kind, FieldKind::List { .. }) {
                self.list_select_down();
                return;
            }
        }
        if self.definition.fields.iter().any(|f| matches!(&f.kind, FieldKind::ScrollableContent { .. })) {
            // Scroll the first scrollable content field (works even when button is selected)
            self.scroll_any_content_down(1);
            return;
        }
        if self.field_list_needs_scroll {
            self.select_field_only(true);
        }
    }

    /// Handle mouse scrollbar click: scroll up by amount (lists or content)
    pub fn mouse_scroll_up_by(&mut self, amount: usize) {
        for field in &mut self.definition.fields {
            match &mut field.kind {
                FieldKind::List { scroll_offset, .. } => {
                    *scroll_offset = scroll_offset.saturating_sub(amount);
                    return;
                }
                FieldKind::ScrollableContent { scroll_offset, .. } => {
                    *scroll_offset = scroll_offset.saturating_sub(amount);
                    return;
                }
                _ => {}
            }
        }
    }

    /// Handle mouse scrollbar click: scroll down by amount (lists or content)
    pub fn mouse_scroll_down_by(&mut self, amount: usize) {
        let actual_height = self.actual_content_height;
        for field in &mut self.definition.fields {
            match &mut field.kind {
                FieldKind::List { items, scroll_offset, visible_height, .. } => {
                    let effective_height = actual_height.unwrap_or(*visible_height);
                    let max_scroll = items.len().saturating_sub(effective_height);
                    *scroll_offset = (*scroll_offset + amount).min(max_scroll);
                    return;
                }
                FieldKind::ScrollableContent { lines, scroll_offset, visible_height } => {
                    let effective_height = actual_height.unwrap_or(*visible_height);
                    let max_scroll = lines.len().saturating_sub(effective_height);
                    *scroll_offset = (*scroll_offset + amount).min(max_scroll);
                    return;
                }
                _ => {}
            }
        }
    }

    /// Scroll any scrollable content field up (regardless of selection)
    fn scroll_any_content_up(&mut self, amount: usize) {
        for field in &mut self.definition.fields {
            if let FieldKind::ScrollableContent { scroll_offset, .. } = &mut field.kind {
                *scroll_offset = scroll_offset.saturating_sub(amount);
                return;
            }
        }
    }

    /// Scroll any scrollable content field down (regardless of selection)
    fn scroll_any_content_down(&mut self, amount: usize) {
        let actual_height = self.actual_content_height;
        for field in &mut self.definition.fields {
            if let FieldKind::ScrollableContent { lines, scroll_offset, visible_height } = &mut field.kind {
                let effective_height = actual_height.unwrap_or(*visible_height);
                let max_scroll = lines.len().saturating_sub(effective_height);
                *scroll_offset = (*scroll_offset + amount).min(max_scroll);
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
    // Grid / Tabs navigation
    // ========================================================================

    /// Move the grid cursor by `(dx, dy)` cells. Finds the first `Grid` field,
    /// mirroring how `list_select_up`/`list_select_down` operate on "the
    /// first list field" rather than taking a `FieldId`.
    ///
    /// Horizontal movement (`dx`) walks the flat cell list in reading order
    /// and crosses row boundaries freely — approved mockup uses a *linear*
    /// cursor for left/right, not a strict 2-D one, so `←` at the first
    /// column of a row lands on the last cell of the *previous* row rather
    /// than getting stuck or changing category. Only the absolute first cell
    /// of the whole grid (`←`) or the absolute last cell (`→`) reports the
    /// edge (returns `true`), so the caller (the console key-handling
    /// wiring) can step to the previous/next category there — and only
    /// there; a user walking a 40-cell result set must not hit a category
    /// change every ten keypresses just from crossing rows.
    ///
    /// Vertical movement (`dy`) is still a real 2-D move: it clamps at the
    /// top/bottom row and never reports an edge — there is no "next
    /// category" to fall through to vertically.
    ///
    /// Returns `false` (no edge hit) if there is no `Grid` field, it is
    /// empty, or the movement was not blocked.
    pub fn grid_move(&mut self, dx: i32, dy: i32) -> bool {
        for field in &mut self.definition.fields {
            if let FieldKind::Grid { cells, selected_index, scroll_offset, columns, visible_rows } = &mut field.kind {
                let columns = (*columns).max(1);
                if cells.is_empty() {
                    return false;
                }
                let row_len = |r: usize| -> usize {
                    cells.len().saturating_sub(r * columns).min(columns)
                };

                let mut hit_edge = false;

                if dx != 0 {
                    let new_index = *selected_index as i32 + dx;
                    if new_index < 0 || new_index >= cells.len() as i32 {
                        hit_edge = true;
                    } else {
                        *selected_index = new_index as usize;
                    }
                }

                if dy != 0 {
                    let total_rows = cells.len().div_ceil(columns);
                    let row = *selected_index / columns;
                    let col = *selected_index % columns;
                    let new_row = (row as i32 + dy).clamp(0, total_rows as i32 - 1) as usize;
                    let new_col = col.min(row_len(new_row).saturating_sub(1));
                    *selected_index = new_row * columns + new_col;
                }

                let row = *selected_index / columns;
                if row < *scroll_offset {
                    *scroll_offset = row;
                } else if *visible_rows > 0 && row >= *scroll_offset + *visible_rows {
                    *scroll_offset = row + 1 - *visible_rows;
                }

                return hit_edge;
            }
        }
        false
    }

    /// Select a specific cell index in the first `Grid` field (clamped to
    /// the cell range), scrolling to keep it visible.
    pub fn grid_select(&mut self, index: usize) {
        for field in &mut self.definition.fields {
            if let FieldKind::Grid { cells, selected_index, scroll_offset, columns, visible_rows } = &mut field.kind {
                if cells.is_empty() {
                    return;
                }
                let columns = (*columns).max(1);
                *selected_index = index.min(cells.len() - 1);
                let row = *selected_index / columns;
                if row < *scroll_offset {
                    *scroll_offset = row;
                } else if *visible_rows > 0 && row >= *scroll_offset + *visible_rows {
                    *scroll_offset = row + 1 - *visible_rows;
                }
                return;
            }
        }
    }

    /// Scroll the grid by `delta` whole pages (in rows of `visible_rows`),
    /// keeping the selection on-screen and within the cell range. A page is
    /// `visible_rows` rows; `delta` may be negative (Page Up) or positive
    /// (Page Down).
    pub fn grid_page(&mut self, delta: i32) {
        for field in &mut self.definition.fields {
            if let FieldKind::Grid { cells, selected_index, scroll_offset, columns, visible_rows } = &mut field.kind {
                if cells.is_empty() || *visible_rows == 0 {
                    return;
                }
                let columns = (*columns).max(1);
                let total_rows = cells.len().div_ceil(columns);
                let max_scroll = total_rows.saturating_sub(*visible_rows);
                let row_delta = delta * *visible_rows as i32;

                let cur_row = *selected_index / columns;
                let col = *selected_index % columns;
                let new_row = (cur_row as i32 + row_delta).clamp(0, total_rows as i32 - 1) as usize;
                let new_row_len = cells.len().saturating_sub(new_row * columns).min(columns);
                let new_col = col.min(new_row_len.saturating_sub(1));
                *selected_index = new_row * columns + new_col;

                let new_scroll = (*scroll_offset as i32 + row_delta).clamp(0, max_scroll as i32) as usize;
                *scroll_offset = new_scroll;
                // Keep the (possibly re-clamped) selection visible even if the
                // page jump alone didn't land the scroll window on it.
                if new_row < *scroll_offset {
                    *scroll_offset = new_row;
                } else if new_row >= *scroll_offset + *visible_rows {
                    *scroll_offset = new_row + 1 - *visible_rows;
                }
                return;
            }
        }
    }

    /// Jump to the first cell of the first `Grid` field.
    pub fn grid_home(&mut self) {
        for field in &mut self.definition.fields {
            if let FieldKind::Grid { cells, selected_index, scroll_offset, .. } = &mut field.kind {
                if cells.is_empty() {
                    return;
                }
                *selected_index = 0;
                *scroll_offset = 0;
                return;
            }
        }
    }

    /// Jump to the last cell of the first `Grid` field.
    pub fn grid_end(&mut self) {
        for field in &mut self.definition.fields {
            if let FieldKind::Grid { cells, selected_index, scroll_offset, columns, visible_rows } = &mut field.kind {
                if cells.is_empty() {
                    return;
                }
                let columns = (*columns).max(1);
                *selected_index = cells.len() - 1;
                let row = *selected_index / columns;
                let total_rows = cells.len().div_ceil(columns);
                *scroll_offset = total_rows.saturating_sub(*visible_rows).min(row);
                return;
            }
        }
    }

    /// Get the currently selected item in a grid field
    pub fn get_selected_grid_item(&self) -> Option<&ListItem> {
        for field in &self.definition.fields {
            if let FieldKind::Grid { cells, selected_index, .. } = &field.kind {
                return cells.get(*selected_index);
            }
        }
        None
    }

    /// Step the active tab of the first `Tabs` field by `delta` (typically
    /// `±1`, e.g. Tab/Shift-Tab or a Grid horizontal edge), wrapping around
    /// at either end so repeated stepping cycles through every category.
    pub fn tabs_step(&mut self, delta: i32) {
        for field in &mut self.definition.fields {
            if let FieldKind::Tabs { labels, selected_index, .. } = &mut field.kind {
                if labels.is_empty() {
                    return;
                }
                let len = labels.len() as i32;
                let new_index = (*selected_index as i32 + delta).rem_euclid(len);
                *selected_index = new_index as usize;
                return;
            }
        }
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
    fn test_cycle_field_buttons_order() {
        let def = create_test_popup();
        let mut state = PopupState::new(def);
        state.open();

        // Starts on first field
        assert!(matches!(state.selected, ElementSelection::Field(FIELD_NAME)));

        // Tab from a field jumps straight to the first button
        assert!(state.cycle_field_buttons());
        assert!(matches!(state.selected, ElementSelection::Button(BTN_SAVE)));

        // Tab from a button goes to the next button
        assert!(state.cycle_field_buttons());
        assert!(matches!(state.selected, ElementSelection::Button(BTN_CANCEL)));

        // Tab from the last button wraps to the first field
        assert!(state.cycle_field_buttons());
        assert!(matches!(state.selected, ElementSelection::Field(FIELD_NAME)));
    }

    #[test]
    fn test_cycle_field_buttons_rev_is_exact_reverse() {
        let def = create_test_popup();
        let mut state = PopupState::new(def);
        state.open();

        // From the first field, Shift+Tab wraps to the last button
        assert!(state.cycle_field_buttons_rev());
        assert!(matches!(state.selected, ElementSelection::Button(BTN_CANCEL)));

        // From a button, Shift+Tab goes to the previous button
        assert!(state.cycle_field_buttons_rev());
        assert!(matches!(state.selected, ElementSelection::Button(BTN_SAVE)));

        // From the first button, Shift+Tab wraps to the last field
        assert!(state.cycle_field_buttons_rev());
        assert!(matches!(state.selected, ElementSelection::Field(FIELD_ENABLED)));
    }

    #[test]
    fn test_next_prev_item_confined_to_fields_when_scroll_needed() {
        let def = create_test_popup();
        let mut state = PopupState::new(def);
        state.open();
        state.field_list_needs_scroll = true;

        assert!(matches!(state.selected, ElementSelection::Field(FIELD_NAME)));

        // Down moves through fields...
        state.next_item();
        assert!(matches!(state.selected, ElementSelection::Field(FIELD_EMAIL)));
        state.next_item();
        assert!(matches!(state.selected, ElementSelection::Field(FIELD_ENABLED)));

        // ...but clamps at the last field instead of wrapping onto a button.
        state.next_item();
        assert!(matches!(state.selected, ElementSelection::Field(FIELD_ENABLED)));

        // Up moves back through fields...
        state.prev_item();
        assert!(matches!(state.selected, ElementSelection::Field(FIELD_EMAIL)));
        state.prev_item();
        assert!(matches!(state.selected, ElementSelection::Field(FIELD_NAME)));

        // ...and clamps at the first field instead of wrapping onto a button.
        state.prev_item();
        assert!(matches!(state.selected, ElementSelection::Field(FIELD_NAME)));
    }

    #[test]
    fn test_next_prev_item_reaches_buttons_when_scroll_not_needed() {
        // Without field_list_needs_scroll (the default), Up/Down still cycle through
        // buttons the old way - only popups with more fields than fit the screen get the
        // confinement behavior.
        let def = create_test_popup();
        let mut state = PopupState::new(def);
        state.open();
        assert!(!state.field_list_needs_scroll);

        state.next_item();
        state.next_item();
        assert!(matches!(state.selected, ElementSelection::Field(FIELD_ENABLED)));
        state.next_item();
        assert!(matches!(state.selected, ElementSelection::Button(BTN_SAVE)));
    }

    #[test]
    fn test_mouse_wheel_confined_to_fields_when_scroll_needed_and_no_list() {
        let def = create_test_popup();
        let mut state = PopupState::new(def);
        state.open();
        state.field_list_needs_scroll = true;

        state.mouse_scroll_down();
        assert!(matches!(state.selected, ElementSelection::Field(FIELD_EMAIL)));
        state.mouse_scroll_down();
        assert!(matches!(state.selected, ElementSelection::Field(FIELD_ENABLED)));
        // Clamps rather than reaching a button.
        state.mouse_scroll_down();
        assert!(matches!(state.selected, ElementSelection::Field(FIELD_ENABLED)));

        state.mouse_scroll_up();
        state.mouse_scroll_up();
        assert!(matches!(state.selected, ElementSelection::Field(FIELD_NAME)));
        // Clamps at the top too.
        state.mouse_scroll_up();
        assert!(matches!(state.selected, ElementSelection::Field(FIELD_NAME)));
    }

    #[test]
    fn test_left_aligned_buttons_ordered_last() {
        let def = PopupDefinition::new(PopupId("test_left"), "Test")
            .with_field(Field::new(FIELD_NAME, "Name", FieldKind::text("")))
            .with_button(Button::new(BTN_SAVE, "Save").primary().with_shortcut('S'))
            .with_button(Button::new(BTN_CANCEL, "Delete").danger().left_align().with_shortcut('D'));
        let mut state = PopupState::new(def);
        state.open();

        // Right-aligned buttons come before left-aligned ones in the order
        assert!(state.cycle_field_buttons()); // field -> first button (right-aligned Save)
        assert!(matches!(state.selected, ElementSelection::Button(BTN_SAVE)));
        assert!(state.cycle_field_buttons()); // Save -> Delete (left-aligned)
        assert!(matches!(state.selected, ElementSelection::Button(BTN_CANCEL)));
        assert!(state.cycle_field_buttons()); // wraps back to field
        assert!(matches!(state.selected, ElementSelection::Field(FIELD_NAME)));
    }

    #[test]
    fn test_left_aligned_group_navigates_in_definition_order_not_tab_index() {
        // Regression test: a left-aligned group must navigate in the same
        // left-to-right order the console renderer draws it in (definition
        // order), even if a later button in that group carries an explicit
        // tab_index. This mirrors World Editor's real layout: the "?" help
        // button is defined/inserted first with no tab_index, "Delete" is
        // defined second with a tab_index — the left group must still be
        // visited "?" then "Delete" (matching what's drawn), not reordered
        // by tab_index.
        const BTN_HELP: ButtonId = ButtonId(99);
        const BTN_DELETE: ButtonId = ButtonId(3);
        let def = PopupDefinition::new(PopupId("test_left_order"), "Test")
            .with_field(Field::new(FIELD_NAME, "Name", FieldKind::text("")))
            .with_button(Button::new(BTN_HELP, "?").left_align()) // defined first, no tab_index
            .with_button(
                Button::new(BTN_DELETE, "Delete")
                    .danger()
                    .left_align()
                    .with_tab_index(3), // defined second, but has a tab_index
            );
        let mut state = PopupState::new(def);
        state.open();

        // No buttons are right-aligned, so Tab from the field jumps straight
        // into the left group in definition/render order: "?" then "Delete".
        assert!(state.cycle_field_buttons());
        assert!(
            matches!(state.selected, ElementSelection::Button(BTN_HELP)),
            "left group must start on the first-defined button (?), not the one with a tab_index"
        );
        assert!(state.cycle_field_buttons());
        assert!(matches!(state.selected, ElementSelection::Button(BTN_DELETE)));
        assert!(state.cycle_field_buttons()); // wraps back to field
        assert!(matches!(state.selected, ElementSelection::Field(FIELD_NAME)));
    }

    #[test]
    fn test_next_item_and_prev_item_full_cycle() {
        let def = create_test_popup();
        let mut state = PopupState::new(def);
        state.open();

        // Down moves through fields in order, then into buttons
        assert!(matches!(state.selected, ElementSelection::Field(FIELD_NAME)));
        state.next_item();
        assert!(matches!(state.selected, ElementSelection::Field(FIELD_EMAIL)));
        state.next_item();
        assert!(matches!(state.selected, ElementSelection::Field(FIELD_ENABLED)));
        state.next_item();
        assert!(matches!(state.selected, ElementSelection::Button(BTN_SAVE)));
        state.next_item();
        assert!(matches!(state.selected, ElementSelection::Button(BTN_CANCEL)));
        state.next_item();
        assert!(matches!(state.selected, ElementSelection::Field(FIELD_NAME))); // wraps

        // Up is the exact reverse
        state.prev_item();
        assert!(matches!(state.selected, ElementSelection::Button(BTN_CANCEL)));
        state.prev_item();
        assert!(matches!(state.selected, ElementSelection::Button(BTN_SAVE)));
        state.prev_item();
        assert!(matches!(state.selected, ElementSelection::Field(FIELD_ENABLED)));
    }

    #[test]
    fn test_find_button_by_shortcut() {
        let def = create_test_popup();
        let state = PopupState::new(def);

        assert_eq!(state.find_button_by_shortcut('s'), Some(BTN_SAVE));
        assert_eq!(state.find_button_by_shortcut('S'), Some(BTN_SAVE));
        assert_eq!(state.find_button_by_shortcut('c'), Some(BTN_CANCEL));
        assert_eq!(state.find_button_by_shortcut('z'), None);
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

    /// Build `n` grid cells, `id`/`columns[0]` = "cell_<i>", 4 columns wide.
    fn grid_cells(n: usize) -> Vec<ListItem> {
        (0..n)
            .map(|i| ListItem {
                id: format!("cell_{i}"),
                columns: vec![format!("cell_{i}")],
                style: ListItemStyle::default(),
            })
            .collect()
    }

    fn grid_popup(n: usize, columns: usize, visible_rows: usize) -> PopupState {
        let def = PopupDefinition::new(PopupId("test"), "Test")
            .with_field(Field::new(FieldId(1), "", FieldKind::grid(grid_cells(n), columns, visible_rows)));
        let mut state = PopupState::new(def);
        state.open();
        state
    }

    fn selected_grid_id(state: &PopupState) -> Option<String> {
        state.get_selected_grid_item().map(|i| i.id.clone())
    }

    #[test]
    fn test_grid_move_horizontal_edges_reported() {
        // 10 cells, 4 columns: rows are [0..4), [4..8), [8..10) (a short last row).
        // Horizontal movement is a LINEAR cursor over the flat cell list — it
        // crosses row boundaries freely, and only the absolute first/last cell
        // of the whole grid reports the edge (approved-mockup behaviour).
        let mut state = grid_popup(10, 4, 10); // tall viewport: no scrolling involved here
        assert_eq!(selected_grid_id(&state), Some("cell_0".into()));

        // Left off cell 0 (the absolute first cell) is the edge.
        assert!(state.grid_move(-1, 0), "left at the absolute first cell must report the edge");
        assert_eq!(selected_grid_id(&state), Some("cell_0".into()), "blocked move must not change selection");

        // Walking right crosses every column, including row boundaries, without
        // ever reporting an edge until the truly last cell.
        for expected in 1..=9 {
            assert!(!state.grid_move(1, 0), "moving right before the last cell must not report an edge");
            assert_eq!(selected_grid_id(&state), Some(format!("cell_{expected}")));
        }

        // Right at the absolute last cell (cell_9, the short last row's only
        // occupied second column) is the edge.
        assert!(state.grid_move(1, 0), "right at the absolute last cell must report the edge");
        assert_eq!(selected_grid_id(&state), Some("cell_9".into()));

        // Walking back left crosses every row boundary too, landing exactly one
        // cell back each time, until the absolute first cell reports the edge.
        for expected in (0..=8).rev() {
            assert!(!state.grid_move(-1, 0), "moving left before the first cell must not report an edge");
            assert_eq!(selected_grid_id(&state), Some(format!("cell_{expected}")));
        }
        assert!(state.grid_move(-1, 0));
        assert_eq!(selected_grid_id(&state), Some("cell_0".into()));
    }

    #[test]
    fn test_grid_move_horizontal_crosses_row_boundaries() {
        // Pins the specific "linear cursor" cases the approved mockup requires:
        // ← from the first cell of a non-first row lands on the last cell of
        // the previous row (not a category change), and the symmetric → case.
        let mut state = grid_popup(10, 4, 10);

        // First cell of row 1 (cell_4) -> left -> last cell of row 0 (cell_3).
        state.grid_select(4);
        assert!(!state.grid_move(-1, 0), "crossing a row boundary leftward must not report the edge");
        assert_eq!(selected_grid_id(&state), Some("cell_3".into()));

        // Last cell of row 0 (cell_3) -> right -> first cell of row 1 (cell_4).
        state.grid_select(3);
        assert!(!state.grid_move(1, 0), "crossing a row boundary rightward must not report the edge");
        assert_eq!(selected_grid_id(&state), Some("cell_4".into()));

        // Same check across the short last row's boundary: first cell of row 2
        // (cell_8) -> left -> last cell of row 1 (cell_7).
        state.grid_select(8);
        assert!(!state.grid_move(-1, 0));
        assert_eq!(selected_grid_id(&state), Some("cell_7".into()));
    }

    #[test]
    fn test_grid_move_vertical_clamps_without_reporting_edge() {
        let mut state = grid_popup(10, 4, 10);
        // Already at the top row: moving up clamps, and dy-only movement never
        // reports an edge (there is no "next category" vertically).
        assert!(!state.grid_move(0, -1));
        assert_eq!(selected_grid_id(&state), Some("cell_0".into()));

        state.grid_select(9); // bottom-right-most cell (short last row)
        assert!(!state.grid_move(0, 1), "moving down from the last row must clamp, not report an edge");
        assert_eq!(selected_grid_id(&state), Some("cell_9".into()));
    }

    #[test]
    fn test_grid_page_clamps_to_content() {
        // 10 cells, 4 columns -> 3 rows; visible_rows=2 -> max_scroll = 1.
        let mut state = grid_popup(10, 4, 2);
        state.grid_page(1); // page down: row 0 -> row 2 (clamped), scroll -> 1
        assert_eq!(selected_grid_id(&state), Some("cell_8".into()));
        if let Some(field) = state.field(FieldId(1)) {
            if let FieldKind::Grid { scroll_offset, .. } = &field.kind {
                assert_eq!(*scroll_offset, 1);
            } else {
                panic!("expected Grid field");
            }
        }

        state.grid_page(-1); // page up: back to row 0, scroll -> 0
        assert_eq!(selected_grid_id(&state), Some("cell_0".into()));
        if let Some(field) = state.field(FieldId(1)) {
            if let FieldKind::Grid { scroll_offset, .. } = &field.kind {
                assert_eq!(*scroll_offset, 0);
            }
        }

        // Paging further up/down than the content has must clamp, not panic or
        // wrap.
        state.grid_page(-5);
        assert_eq!(selected_grid_id(&state), Some("cell_0".into()));
        state.grid_page(5);
        assert_eq!(selected_grid_id(&state), Some("cell_8".into()));
    }

    #[test]
    fn test_grid_home_and_end() {
        let mut state = grid_popup(10, 4, 2);
        state.grid_select(5);
        state.grid_home();
        assert_eq!(selected_grid_id(&state), Some("cell_0".into()));
        if let Some(field) = state.field(FieldId(1)) {
            if let FieldKind::Grid { scroll_offset, .. } = &field.kind {
                assert_eq!(*scroll_offset, 0);
            }
        }

        state.grid_end();
        assert_eq!(selected_grid_id(&state), Some("cell_9".into()));
        if let Some(field) = state.field(FieldId(1)) {
            if let FieldKind::Grid { scroll_offset, .. } = &field.kind {
                // 3 rows total, 2 visible -> scrolled to show the last row.
                assert_eq!(*scroll_offset, 1);
            }
        }
    }

    #[test]
    fn test_next_item_prev_item_stay_inside_grid() {
        let mut state = grid_popup(10, 4, 10);
        state.next_item(); // Down: row0 -> row1, same column
        assert_eq!(selected_grid_id(&state), Some("cell_4".into()));
        state.prev_item(); // Up: back to row0
        assert_eq!(selected_grid_id(&state), Some("cell_0".into()));
    }

    #[test]
    fn test_tabs_step_wraps_both_directions() {
        let labels = vec!["All".to_string(), "Smileys".to_string(), "People".to_string(), "Nature".to_string()];
        let def = PopupDefinition::new(PopupId("test"), "Test")
            .with_field(Field::new(FieldId(1), "", FieldKind::tabs(labels, 0)));
        let mut state = PopupState::new(def);
        state.open();

        let selected_tab = |state: &PopupState| -> usize {
            match state.field(FieldId(1)).map(|f| &f.kind) {
                Some(FieldKind::Tabs { selected_index, .. }) => *selected_index,
                _ => panic!("expected Tabs field"),
            }
        };

        assert_eq!(selected_tab(&state), 0);
        state.tabs_step(1);
        assert_eq!(selected_tab(&state), 1);
        state.tabs_step(1);
        state.tabs_step(1);
        assert_eq!(selected_tab(&state), 3);
        state.tabs_step(1); // wraps past the last tab back to the first
        assert_eq!(selected_tab(&state), 0);
        state.tabs_step(-1); // wraps the other way
        assert_eq!(selected_tab(&state), 3);
    }

    #[test]
    fn test_grid_and_tabs_field_kind_flags() {
        let grid = FieldKind::grid(grid_cells(3), 3, 2);
        assert!(grid.is_interactive());
        assert!(!grid.is_text_editable());

        let tabs = FieldKind::tabs(vec!["All".to_string()], 0);
        assert!(tabs.is_interactive());
        assert!(!tabs.is_text_editable());
    }
}
