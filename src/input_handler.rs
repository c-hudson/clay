//! Input handling and mouse event functions extracted from main.rs

use crossterm::{
    event::{KeyCode, KeyEvent, KeyModifiers, DisableMouseCapture},
    execute,
};
use std::sync::atomic::Ordering;
use std::time::Duration;

use crate::{
    popup, persistence, keybindings,
    SpellChecker,
    WsMessage,
    Theme, WorldSwitchMode,
    Encoding, AutoConnectType, KeepAliveType, WorldType,
    current_timestamp_secs,
    App, World, EditorFocus, EditorSide, DEBUG_ENABLED,
    handle_new_popup_key, NewPopupAction,
    WorldSelectorAction, ActionsListAction, NotesListAction,
    web_settings_from_custom_data, apply_web_settings,
};

/// Save editor content (to file or world notes) and close editor
pub(crate) fn save_editor_content(app: &mut App) -> KeyAction {
    if let Some(ref path) = app.editor.file_path {
        // Save to file
        match std::fs::write(path, &app.editor.buffer) {
            Ok(()) => {
                app.add_output(&format!("Saved: {}", path.display()));
            }
            Err(e) => {
                app.add_output(&format!("Failed to save file: {}", e));
                return KeyAction::None; // Don't close on error
            }
        }
    } else if let Some(world_idx) = app.editor.world_index {
        // Save to world notes
        if world_idx < app.worlds.len() {
            app.worlds[world_idx].settings.notes = app.editor.buffer.clone();
            // Save settings to persist notes
            let _ = persistence::save_settings(app);
            app.add_output("Notes saved.");
        }
    }
    app.editor.close();
    app.needs_output_redraw = true;
    app.needs_terminal_clear = true;
    KeyAction::None
}

pub(crate) enum KeyAction {
    Quit,
    SendCommand(String),
    Connect, // Trigger connection from settings popup
    Redraw,  // Force screen redraw
    Reload,  // Trigger /reload
    Suspend, // Ctrl+Z to suspend process
    SwitchedWorld(usize), // Console switched to this world, broadcast unseen clear
    None,
}

/// Handle a mouse click on a popup. Returns true if a button was activated (caller should
/// then call handle_new_popup_key with Enter to trigger the button action).
pub(crate) fn handle_popup_mouse_click(app: &mut App, column: u16, row: u16) -> bool {
    if let Some(state) = app.popup_manager.current_mut() {
        for (rect, element) in &state.hit_areas {
            if column >= rect.x && column < rect.x + rect.width
                && row >= rect.y && row < rect.y + rect.height
            {
                match element {
                    popup::ElementSelection::Field(field_id) => {
                        let field_id = *field_id;
                        // If currently editing a different field, commit the edit first
                        if state.editing {
                            if let popup::ElementSelection::Field(current_id) = &state.selected {
                                if *current_id != field_id {
                                    state.commit_edit();
                                }
                            }
                        }
                        state.selected = popup::ElementSelection::Field(field_id);
                        // Toggle or enter edit based on field type
                        if let Some(field) = state.definition.fields.iter().find(|f| f.id == field_id) {
                            match &field.kind {
                                popup::FieldKind::Toggle { .. } => {
                                    state.toggle_current();
                                }
                                popup::FieldKind::Select { .. } => {
                                    state.cycle_selected();
                                }
                                popup::FieldKind::Text { .. } | popup::FieldKind::MultilineText { .. } => {
                                    if !state.editing {
                                        state.start_edit();
                                    }
                                }
                                _ => {}
                            }
                        }
                        return false;
                    }
                    popup::ElementSelection::Button(button_id) => {
                        let button_id = *button_id;
                        if state.editing {
                            state.commit_edit();
                        }
                        state.selected = popup::ElementSelection::Button(button_id);
                        return true; // Caller should trigger button action
                    }
                    popup::ElementSelection::None => {}
                }
            }
        }
    }
    false
}

/// Map screen coordinates to a content line in a popup content area.
/// Returns (field_id, content_line_index) if the click is within a content area.
pub(crate) fn screen_to_content_line(app: &App, column: u16, row: u16) -> Option<(popup::FieldId, usize)> {
    if let Some(state) = app.popup_manager.current() {
        for ca in &state.content_areas {
            if column >= ca.area.x && column < ca.area.x + ca.area.width
                && row >= ca.area.y && row < ca.area.y + ca.area.height
            {
                let line = ca.scroll_offset + (row - ca.area.y) as usize;
                if line < ca.total_lines {
                    return Some((ca.field_id, line));
                }
            }
        }
    }
    None
}

/// Start a mouse highlight on mouse down in a content area.
/// Returns true if the click was in a content area (highlight started or scrollbar clicked).
pub(crate) fn handle_popup_mouse_highlight_start(app: &mut App, column: u16, row: u16) -> bool {
    // Check for scrollbar clicks first (rightmost column of content areas with overflow)
    if let Some(state) = app.popup_manager.current() {
        for ca in &state.content_areas {
            let scrollbar_x = ca.area.x + ca.area.width.saturating_sub(1);
            if column == scrollbar_x
                && row >= ca.area.y && row < ca.area.y + ca.area.height
                && ca.total_lines > ca.area.height as usize
            {
                let visible = ca.area.height as usize;
                let total = ca.total_lines;
                let max_scroll = total.saturating_sub(visible);
                let scroll = ca.scroll_offset;
                let thumb_size = (visible as f64 / total as f64 * visible as f64).max(1.0) as usize;
                let thumb_pos = if max_scroll == 0 { 0 } else {
                    (scroll as f64 / max_scroll as f64 * (visible - thumb_size) as f64) as usize
                };
                let click_row = (row - ca.area.y) as usize;
                let page = visible.saturating_sub(1).max(1);
                if click_row < thumb_pos {
                    let state = app.popup_manager.current_mut().unwrap();
                    state.mouse_scroll_up_by(page);
                } else if click_row >= thumb_pos + thumb_size {
                    let state = app.popup_manager.current_mut().unwrap();
                    state.mouse_scroll_down_by(page);
                }
                return true;
            }
        }
    }

    if let Some((field_id, line)) = screen_to_content_line(app, column, row) {
        if let Some(state) = app.popup_manager.current_mut() {
            // Select this field
            state.selected = popup::ElementSelection::Field(field_id);

            // For list fields, also update the list's selected_index
            if let Some(field) = state.definition.fields.iter_mut().find(|f| f.id == field_id) {
                if let popup::FieldKind::List { items, selected_index, .. } = &mut field.kind {
                    if line < items.len() {
                        *selected_index = line;
                    }
                }
            }

            state.highlight = Some(popup::PopupHighlight {
                field_id,
                start_line: line,
                end_line: line,
                dragging: true,
            });
        }
        return true;
    }
    // Click was not in a content area - clear any existing highlight
    if let Some(state) = app.popup_manager.current_mut() {
        state.highlight = None;
    }
    false
}

/// Extend a mouse highlight on drag.
pub(crate) fn handle_popup_mouse_highlight_drag(app: &mut App, _column: u16, row: u16) {
    // Check if we're currently dragging
    let is_dragging = app.popup_manager.current()
        .and_then(|s| s.highlight.as_ref())
        .is_some_and(|h| h.dragging);
    if !is_dragging {
        return;
    }

    // Get the field_id we're highlighting
    let highlight_field = app.popup_manager.current()
        .and_then(|s| s.highlight.as_ref())
        .map(|h| h.field_id);
    let Some(field_id) = highlight_field else { return };

    // Find the content area for this field and clamp the row
    let content_line = app.popup_manager.current().and_then(|state| {
        state.content_areas.iter().find(|ca| ca.field_id == field_id).map(|ca| {
            let clamped_row = row.max(ca.area.y).min(ca.area.y + ca.area.height.saturating_sub(1));
            let line = ca.scroll_offset + (clamped_row - ca.area.y) as usize;
            line.min(ca.total_lines.saturating_sub(1))
        })
    });

    if let Some(line) = content_line {
        if let Some(state) = app.popup_manager.current_mut() {
            if let Some(highlight) = &mut state.highlight {
                highlight.end_line = line;
            }
        }
    }
}

/// End a mouse highlight on mouse up.
pub(crate) fn handle_popup_mouse_highlight_end(app: &mut App) {
    if let Some(state) = app.popup_manager.current_mut() {
        if let Some(highlight) = &mut state.highlight {
            highlight.dragging = false;
            // If start == end (single click), clear the highlight
            if highlight.start_line == highlight.end_line {
                state.highlight = None;
            }
        }
    }
}

pub(crate) fn handle_key_event(key: KeyEvent, app: &mut App) -> KeyAction {

    // Handle confirm dialog first (highest priority)
    if app.confirm_dialog.visible {
        match key.code {
            KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down | KeyCode::Tab => {
                // Toggle between Yes and No
                app.confirm_dialog.yes_selected = !app.confirm_dialog.yes_selected;
            }
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                app.confirm_dialog.yes_selected = true;
            }
            KeyCode::Char('n') | KeyCode::Char('N') => {
                app.confirm_dialog.yes_selected = false;
            }
            KeyCode::Enter => {
                // ConfirmAction::None is the only variant now
                app.confirm_dialog.close();
            }
            KeyCode::Esc => {
                // Cancel - just close the dialog
                app.confirm_dialog.close();
            }
            _ => {}
        }
        return KeyAction::None;
    }

    // Handle split-screen editor (before popups, after confirm dialog)
    if app.editor.visible {
        // Ctrl+Space toggles focus between editor and input
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char(' ') {
            app.editor.toggle_focus();
            return KeyAction::None;
        }

        // When editor is focused, handle editor keys
        if app.editor.focus == EditorFocus::Editor {
            match key.code {
                KeyCode::Esc => {
                    // Close editor without saving
                    app.editor.close();
                    app.needs_output_redraw = true;
                    app.needs_terminal_clear = true;
                    return KeyAction::None;
                }
                KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    // Save and close
                    return save_editor_content(app);
                }
                KeyCode::Char('s') | KeyCode::Char('S') if !key.modifiers.contains(KeyModifiers::CONTROL) && !key.modifiers.contains(KeyModifiers::ALT) => {
                    // Just 'S' key - check if at start of buffer (for shortcut)
                    // Actually, we want S anywhere to be a shortcut when not typing
                    // Let's make S work as a shortcut only without any modifiers
                    // But we should insert 's' when typing... let's just use Ctrl+S
                    // Insert the character
                    app.editor.insert_char('s');
                    return KeyAction::None;
                }
                KeyCode::Char('c') | KeyCode::Char('C') if !key.modifiers.contains(KeyModifiers::CONTROL) && !key.modifiers.contains(KeyModifiers::ALT) => {
                    // Insert the character
                    app.editor.insert_char(if key.code == KeyCode::Char('C') { 'C' } else { 'c' });
                    return KeyAction::None;
                }
                KeyCode::Up => {
                    let editor_width = (app.output_width / 2).saturating_sub(2) as usize;
                    app.editor.cursor_up(editor_width);
                    let visible_lines = app.output_height.saturating_sub(2) as usize;
                    app.editor.ensure_cursor_visible(visible_lines, editor_width);
                    return KeyAction::None;
                }
                KeyCode::Down => {
                    let editor_width = (app.output_width / 2).saturating_sub(2) as usize;
                    app.editor.cursor_down(editor_width);
                    let visible_lines = app.output_height.saturating_sub(2) as usize;
                    app.editor.ensure_cursor_visible(visible_lines, editor_width);
                    return KeyAction::None;
                }
                KeyCode::Left => {
                    app.editor.cursor_left();
                    let visible_lines = app.output_height.saturating_sub(2) as usize;
                    let editor_width = (app.output_width / 2).saturating_sub(2) as usize;
                    app.editor.ensure_cursor_visible(visible_lines, editor_width);
                    return KeyAction::None;
                }
                KeyCode::Right => {
                    app.editor.cursor_right();
                    let visible_lines = app.output_height.saturating_sub(2) as usize;
                    let editor_width = (app.output_width / 2).saturating_sub(2) as usize;
                    app.editor.ensure_cursor_visible(visible_lines, editor_width);
                    return KeyAction::None;
                }
                KeyCode::Home => {
                    app.editor.cursor_home();
                    let visible_lines = app.output_height.saturating_sub(2) as usize;
                    let editor_width = (app.output_width / 2).saturating_sub(2) as usize;
                    app.editor.ensure_cursor_visible(visible_lines, editor_width);
                    return KeyAction::None;
                }
                KeyCode::End => {
                    app.editor.cursor_end();
                    let visible_lines = app.output_height.saturating_sub(2) as usize;
                    let editor_width = (app.output_width / 2).saturating_sub(2) as usize;
                    app.editor.ensure_cursor_visible(visible_lines, editor_width);
                    return KeyAction::None;
                }
                KeyCode::PageUp => {
                    let visible_lines = app.output_height.saturating_sub(2) as usize;
                    let editor_width = (app.output_width / 2).saturating_sub(2) as usize;
                    app.editor.page_up(visible_lines, editor_width);
                    return KeyAction::None;
                }
                KeyCode::PageDown => {
                    let visible_lines = app.output_height.saturating_sub(2) as usize;
                    let editor_width = (app.output_width / 2).saturating_sub(2) as usize;
                    app.editor.page_down(visible_lines, editor_width);
                    return KeyAction::None;
                }
                KeyCode::Enter => {
                    app.editor.insert_char('\n');
                    let visible_lines = app.output_height.saturating_sub(2) as usize;
                    let editor_width = (app.output_width / 2).saturating_sub(2) as usize;
                    app.editor.ensure_cursor_visible(visible_lines, editor_width);
                    return KeyAction::None;
                }
                KeyCode::Backspace => {
                    app.editor.delete_backward();
                    let visible_lines = app.output_height.saturating_sub(2) as usize;
                    let editor_width = (app.output_width / 2).saturating_sub(2) as usize;
                    app.editor.ensure_cursor_visible(visible_lines, editor_width);
                    return KeyAction::None;
                }
                KeyCode::Delete => {
                    app.editor.delete_forward();
                    return KeyAction::None;
                }
                KeyCode::Tab => {
                    // Insert 4 spaces for tab
                    for _ in 0..4 {
                        app.editor.insert_char(' ');
                    }
                    let visible_lines = app.output_height.saturating_sub(2) as usize;
                    let editor_width = (app.output_width / 2).saturating_sub(2) as usize;
                    app.editor.ensure_cursor_visible(visible_lines, editor_width);
                    return KeyAction::None;
                }
                KeyCode::Char(c) => {
                    // Insert character
                    let ch = if key.modifiers.contains(KeyModifiers::SHIFT) {
                        c.to_ascii_uppercase()
                    } else {
                        c
                    };
                    app.editor.insert_char(ch);
                    let visible_lines = app.output_height.saturating_sub(2) as usize;
                    let editor_width = (app.output_width / 2).saturating_sub(2) as usize;
                    app.editor.ensure_cursor_visible(visible_lines, editor_width);
                    return KeyAction::None;
                }
                _ => {}
            }
            return KeyAction::None;
        }
        // When input is focused, fall through to normal input handling below
    }

    // Handle new unified popup system (help popup and others)
    if app.has_new_popup() {
        match handle_new_popup_key(app, key) {
            NewPopupAction::Command(cmd) => {
                // Web-only editors: show URL hint in console
                if cmd == "/theme-editor" || cmd == "/keybind-editor" {
                    let page = cmd.trim_start_matches('/');
                    let proto = if app.settings.web_secure { "https" } else { "http" };
                    if app.settings.http_enabled {
                        app.add_output(&format!("Open in browser: {}://localhost:{}/{}", proto, app.settings.http_port, page));
                    } else {
                        app.add_output(&format!("Enable HTTP in /web settings, then open /{} in a browser.", page));
                    }
                    return KeyAction::None;
                }
                // Menu command selected - execute it
                return KeyAction::SendCommand(cmd);
            }
            NewPopupAction::Confirm(data) => {
                // Handle confirmed action
                if let Some(world_index_str) = data.get("world_index") {
                    if let Ok(world_index) = world_index_str.parse::<usize>() {
                        // Delete the world
                        if app.worlds.len() > 1 && world_index < app.worlds.len() {
                            let world_name = app.worlds[world_index].name.clone();
                            app.worlds.remove(world_index);
                            // Adjust current_world_index if needed
                            if app.current_world_index >= app.worlds.len() {
                                app.current_world_index = app.worlds.len().saturating_sub(1);
                            } else if app.current_world_index > world_index {
                                app.current_world_index -= 1;
                            }
                            // Adjust previous_world_index if needed
                            if let Some(prev) = app.previous_world_index {
                                if prev >= app.worlds.len() {
                                    app.previous_world_index = Some(app.worlds.len().saturating_sub(1));
                                } else if prev > world_index {
                                    app.previous_world_index = Some(prev - 1);
                                }
                            }
                            app.add_output(&format!("World '{}' deleted.", world_name));
                            // Reopen world selector to show updated list
                            app.open_world_selector_new();
                        }
                    }
                } else if let Some(action_index_str) = data.get("action_index") {
                    if let Ok(action_index) = action_index_str.parse::<usize>() {
                        // Delete the action
                        if action_index < app.settings.actions.len() {
                            let action_name = app.settings.actions[action_index].name.clone();
                            app.settings.actions.remove(action_index);
                            app.add_output(&format!("Action '{}' deleted.", action_name));
                            // Save settings to disk
                            let _ = persistence::save_settings(app);
                            // Reopen actions list to show updated list
                            app.open_actions_list_popup();
                        }
                    }
                } else if data.contains_key("web_save") {
                    // Allow list wildcard warning confirmed — apply settings
                    let settings = web_settings_from_custom_data(&data);
                    apply_web_settings(app, &settings);
                }
            }
            NewPopupAction::ConfirmCancelled(data) => {
                // Reopen the parent list popup when confirm dialog is cancelled
                if data.contains_key("world_index") {
                    app.open_world_selector_new();
                } else if data.contains_key("action_index") {
                    app.open_actions_list_popup();
                }
            }
            NewPopupAction::WorldSelector(action) => {
                match action {
                    WorldSelectorAction::Connect(name) => {
                        // Find world and connect to it
                        if let Some(idx) = app.find_world(&name) {
                            app.switch_world(idx);
                            if !app.current_world().connected {
                                if app.current_world().settings.has_connection_settings() {
                                    return KeyAction::SendCommand("/__connect".to_string());
                                } else {
                                    app.add_output(&format!("World '{}' has no connection settings.", name));
                                }
                            }
                        }
                    }
                    WorldSelectorAction::Edit(name) => {
                        // Open world editor using new popup
                        if let Some(idx) = app.find_world(&name) {
                            app.open_world_editor_popup_new(idx);
                        }
                    }
                    WorldSelectorAction::Delete(name) => {
                        // Open confirmation dialog for delete
                        if let Some(idx) = app.find_world(&name) {
                            if app.worlds.len() > 1 {
                                app.open_delete_world_confirm(&name, idx);
                            } else {
                                app.add_output("Cannot delete the last world.");
                            }
                        }
                    }
                    WorldSelectorAction::Add => {
                        // Create new world and open editor using new popup
                        let new_name = format!("World {}", app.worlds.len() + 1);
                        let new_world = World::new(&new_name);
                        app.worlds.push(new_world);
                        let idx = app.worlds.len() - 1;
                        app.open_world_editor_popup_new(idx);
                    }
                }
            }
            NewPopupAction::WorldSelectorFilter => {
                // Filter changed - update the world list
                use popup::definitions::world_selector::SELECTOR_FIELD_FILTER;
                if let Some(state) = app.popup_manager.current_mut() {
                    // Use edit_buffer if currently editing, otherwise use field value
                    let filter_text = if state.editing && state.is_field_selected(SELECTOR_FIELD_FILTER) {
                        state.edit_buffer.clone()
                    } else {
                        state.get_text(SELECTOR_FIELD_FILTER).unwrap_or("").to_string()
                    };
                    // Build world info list
                    let all_worlds: Vec<popup::definitions::world_selector::WorldInfo> = app.worlds
                        .iter()
                        .enumerate()
                        .map(|(idx, w)| popup::definitions::world_selector::WorldInfo {
                            name: w.name.clone(),
                            hostname: w.settings.hostname.clone(),
                            port: w.settings.port.to_string(),
                            user: w.settings.user.clone(),
                            is_connected: w.connected,
                            is_current: idx == app.current_world_index,
                        })
                        .collect();
                    // Apply filter
                    let filtered = popup::definitions::world_selector::filter_worlds(&all_worlds, &filter_text);
                    // Update the list in the popup state
                    popup::definitions::world_selector::update_world_list(state, &filtered);
                }
            }
            NewPopupAction::SetupSaved(settings) => {
                // Apply saved settings
                app.settings.more_mode_enabled = settings.more_mode;
                app.settings.spell_check_enabled = settings.spell_check;
                app.settings.temp_convert_enabled = settings.temp_convert;
                app.settings.world_switch_mode = if settings.world_switching == "unseen_first" {
                    WorldSwitchMode::UnseenFirst
                } else {
                    WorldSwitchMode::Alphabetical
                };
                app.settings.debug_enabled = settings.debug;
                DEBUG_ENABLED.store(settings.debug, Ordering::Relaxed);
                // Note: show_tags is not in setup anymore - controlled by F2 or /tag
                app.input_height = settings.input_height as u16;
                app.settings.gui_theme = Theme::from_name(&settings.gui_theme);
                app.settings.tls_proxy_enabled = settings.tls_proxy;
                if app.settings.dictionary_path != settings.dictionary_path {
                    app.settings.dictionary_path = settings.dictionary_path.clone();
                    app.spell_checker = SpellChecker::new(&app.settings.dictionary_path);
                }
                app.settings.editor_side = EditorSide::from_name(&settings.editor_side);
                // Update mouse setting; if disabled, turn off capture immediately
                app.settings.mouse_enabled = settings.mouse_enabled;
                if !settings.mouse_enabled && app.mouse_capture_active {
                    let _ = execute!(std::io::stdout(), DisableMouseCapture);
                    app.mouse_capture_active = false;
                }
                app.settings.zwj_enabled = settings.zwj_enabled;
                app.settings.ansi_music_enabled = settings.ansi_music;
                app.settings.new_line_indicator = settings.new_line_indicator;
                let old_tts_mode = app.settings.tts_mode;
                app.settings.tts_mode = crate::tts::TtsMode::from_name(&settings.tts_mode);
                app.settings.tts_speak_mode = crate::tts::TtsSpeakMode::from_name(&settings.tts_speak_mode);
                // Auto-unmute when TTS is enabled from Off
                if old_tts_mode == crate::tts::TtsMode::Off && app.settings.tts_mode != crate::tts::TtsMode::Off {
                    app.settings.tts_muted = false;
                }
                // Save settings to disk
                let _ = persistence::save_settings(app);
            }
            NewPopupAction::WebSaved(settings) => {
                // Check for wildcard '*' in allow list — warn user
                if crate::websocket::allow_list_has_wildcard(&settings.ws_allow_list) {
                    use popup::definitions::confirm::create_allow_list_warning_dialog;
                    let mut def = create_allow_list_warning_dialog();
                    // Store settings in custom_data for retrieval on confirm
                    def.custom_data.insert("web_save".to_string(), "1".to_string());
                    def.custom_data.insert("web_secure".to_string(), settings.web_secure.to_string());
                    def.custom_data.insert("http_enabled".to_string(), settings.http_enabled.to_string());
                    def.custom_data.insert("http_port".to_string(), settings.http_port);
                    def.custom_data.insert("ws_password".to_string(), settings.ws_password);
                    def.custom_data.insert("ws_allow_list".to_string(), settings.ws_allow_list);
                    def.custom_data.insert("ws_cert_file".to_string(), settings.ws_cert_file);
                    def.custom_data.insert("ws_key_file".to_string(), settings.ws_key_file);
                    app.popup_manager.open(def);
                } else {
                    apply_web_settings(app, &settings);
                }
            }
            NewPopupAction::ConnectionsClose => {
                // Nothing to do, popup is already closed
            }
            NewPopupAction::ActionsList(action) => {
                match action {
                    ActionsListAction::Add => {
                        // Open action editor for new action
                        app.open_action_editor_popup(None);
                    }
                    ActionsListAction::Edit(idx) => {
                        // Open action editor for existing action
                        if idx < app.settings.actions.len() {
                            app.open_action_editor_popup(Some(idx));
                        }
                    }
                    ActionsListAction::Delete(idx) => {
                        // Open confirmation dialog for delete
                        if idx < app.settings.actions.len() {
                            let name = app.settings.actions[idx].name.clone();
                            app.open_delete_action_confirm(&name, idx);
                        }
                    }
                    ActionsListAction::Toggle(idx) => {
                        // Toggle enable/disable for the action
                        if idx < app.settings.actions.len() {
                            app.settings.actions[idx].enabled = !app.settings.actions[idx].enabled;
                            app.settings.actions[idx].compile_regex();
                            let _ = persistence::save_settings(app);

                            // Update the list display in the popup
                            use popup::definitions::actions::{ActionInfo, filter_actions, ACTIONS_FIELD_FILTER, ACTIONS_FIELD_LIST};
                            if let Some(state) = app.popup_manager.current_mut() {
                                // Get current filter text
                                let filter_text = if state.editing && state.is_field_selected(ACTIONS_FIELD_FILTER) {
                                    state.edit_buffer.clone()
                                } else {
                                    state.get_text(ACTIONS_FIELD_FILTER).unwrap_or("").to_string()
                                };
                                // Build action info list
                                let all_actions: Vec<ActionInfo> = app.settings.actions
                                    .iter()
                                    .enumerate()
                                    .map(|(i, a)| ActionInfo {
                                        name: a.name.clone(),
                                        world: a.world.clone(),
                                        pattern: a.pattern.clone(),
                                        enabled: a.enabled,
                                        index: i,
                                    })
                                    .collect();
                                // Apply filter and sort alphabetically
                                let mut filtered = filter_actions(&all_actions, &filter_text);
                                filtered.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
                                // Update the list in the popup state
                                if let Some(field) = state.field_mut(ACTIONS_FIELD_LIST) {
                                    if let popup::FieldKind::List { items, .. } = &mut field.kind {
                                        // Rebuild items with indices
                                        *items = filtered.iter().map(|info| {
                                                #[cfg(not(windows))]
                                                let status = if info.enabled { "[✓]" } else { "[ ]" };
                                                #[cfg(windows)]
                                                let status = if info.enabled { "[x]" } else { "[ ]" };
                                                let world_part = if info.world.is_empty() {
                                                    String::new()
                                                } else {
                                                    format!("({})", info.world)
                                                };
                                                let pattern_preview = if info.pattern.len() > 30 {
                                                    format!("{}...", &info.pattern[..27])
                                                } else {
                                                    info.pattern.clone()
                                                };
                                                popup::ListItem {
                                                    id: info.index.to_string(),
                                                    columns: vec![
                                                        format!("{} {}", status, info.name),
                                                        world_part,
                                                        pattern_preview,
                                                    ],
                                                    style: popup::ListItemStyle {
                                                        is_disabled: !info.enabled,
                                                        ..Default::default()
                                                    },
                                                }
                                        }).collect();
                                    }
                                }
                            }
                        }
                    }
                }
            }
            NewPopupAction::ActionsListFilter => {
                // Filter changed - update the actions list
                use popup::definitions::actions::{ActionInfo, filter_actions, ACTIONS_FIELD_FILTER};
                if let Some(state) = app.popup_manager.current_mut() {
                    // Use edit_buffer if currently editing, otherwise use field value
                    let filter_text = if state.editing && state.is_field_selected(ACTIONS_FIELD_FILTER) {
                        state.edit_buffer.clone()
                    } else {
                        state.get_text(ACTIONS_FIELD_FILTER).unwrap_or("").to_string()
                    };
                    // Build action info list
                    let all_actions: Vec<ActionInfo> = app.settings.actions
                        .iter()
                        .enumerate()
                        .map(|(i, a)| ActionInfo {
                            name: a.name.clone(),
                            world: a.world.clone(),
                            pattern: a.pattern.clone(),
                            enabled: a.enabled,
                            index: i,
                        })
                        .collect();
                    // Apply filter and sort alphabetically
                    let mut filtered = filter_actions(&all_actions, &filter_text);
                    filtered.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
                    // Update the list in the popup state
                    if let Some(field) = state.field_mut(popup::definitions::actions::ACTIONS_FIELD_LIST) {
                        if let popup::FieldKind::List { items, selected_index, scroll_offset, .. } = &mut field.kind {
                            let old_len = items.len();
                            // Rebuild items with indices
                            *items = filtered.iter().map(|info| {
                                    #[cfg(not(windows))]
                                    let status = if info.enabled { "[✓]" } else { "[ ]" };
                                    #[cfg(windows)]
                                    let status = if info.enabled { "[x]" } else { "[ ]" };
                                    let world_part = if info.world.is_empty() {
                                        String::new()
                                    } else {
                                        format!("({})", info.world)
                                    };
                                    let pattern_preview = if info.pattern.len() > 30 {
                                        format!("{}...", &info.pattern[..27])
                                    } else {
                                        info.pattern.clone()
                                    };
                                    popup::ListItem {
                                        id: info.index.to_string(),  // Store original index as ID
                                        columns: vec![
                                            format!("{} {}", status, info.name),
                                            world_part,
                                            pattern_preview,
                                        ],
                                        style: popup::ListItemStyle {
                                            is_disabled: !info.enabled,
                                            ..Default::default()
                                        },
                                    }
                            }).collect();
                            // Reset selection if list changed significantly
                            if items.is_empty() {
                                *selected_index = 0;
                                *scroll_offset = 0;
                            } else if *selected_index >= items.len() {
                                *selected_index = items.len().saturating_sub(1);
                            }
                            if old_len != items.len() {
                                *scroll_offset = 0;
                            }
                        }
                    }
                }
            }
            NewPopupAction::ActionEditorSave { action, editing_index } => {
                // Validate action
                if action.name.trim().is_empty() {
                    app.add_output("Action name cannot be empty.");
                } else {
                    // Check for duplicate names (case-insensitive)
                    let name_lower = action.name.to_lowercase();
                    let is_duplicate = app.settings.actions.iter().enumerate().any(|(i, a)| {
                        a.name.to_lowercase() == name_lower && Some(i) != editing_index
                    });
                    if is_duplicate {
                        app.add_output(&format!("An action named '{}' already exists.", action.name));
                    } else {
                        // Save the action
                        if let Some(idx) = editing_index {
                            if idx < app.settings.actions.len() {
                                app.settings.actions[idx] = action.clone();
                                app.settings.actions[idx].compile_regex();
                                app.add_output(&format!("Action '{}' updated.", action.name));
                            }
                        } else {
                            app.settings.actions.push(action.clone());
                            app.settings.actions.last_mut().unwrap().compile_regex();
                            app.add_output(&format!("Action '{}' created.", action.name));
                        }
                        // Save settings to disk
                        let _ = persistence::save_settings(app);
                        // Reopen actions list to show updated list
                        app.open_actions_list_popup();
                    }
                }
            }
            NewPopupAction::ActionEditorDelete { editing_index } => {
                if editing_index < app.settings.actions.len() {
                    let name = app.settings.actions[editing_index].name.clone();
                    app.open_delete_action_confirm(&name, editing_index);
                }
            }
            NewPopupAction::WorldEditorSaved(settings) => {
                let idx = settings.world_index;
                if idx < app.worlds.len() {
                    // Update world name
                    app.worlds[idx].name = settings.name;

                    // Update world type
                    app.worlds[idx].settings.world_type = WorldType::from_name(&settings.world_type);

                    // Update MUD settings
                    app.worlds[idx].settings.hostname = settings.hostname;
                    app.worlds[idx].settings.port = settings.port;
                    app.worlds[idx].settings.user = settings.user;
                    app.worlds[idx].settings.password = settings.password;
                    app.worlds[idx].settings.use_ssl = settings.use_ssl;
                    app.worlds[idx].settings.log_enabled = settings.log_enabled;

                    // Update encoding
                    app.worlds[idx].settings.encoding = Encoding::from_name(&settings.encoding);

                    // Update auto connect type
                    app.worlds[idx].settings.auto_connect_type = match settings.auto_connect.as_str() {
                        "prompt" => AutoConnectType::Prompt,
                        "moo_prompt" => AutoConnectType::MooPrompt,
                        "none" => AutoConnectType::NoLogin,
                        _ => AutoConnectType::Connect,
                    };

                    // Update keep alive type
                    app.worlds[idx].settings.keep_alive_type = match settings.keep_alive.as_str() {
                        "none" => KeepAliveType::None,
                        "custom" => KeepAliveType::Custom,
                        "generic" => KeepAliveType::Generic,
                        _ => KeepAliveType::Nop,
                    };
                    app.worlds[idx].settings.keep_alive_cmd = settings.keep_alive_cmd;
                    app.worlds[idx].settings.gmcp_packages = settings.gmcp_packages;
                    let (ar_secs, ar_on_web) = crate::WorldSettings::parse_auto_reconnect(&settings.auto_reconnect_secs);
                    app.worlds[idx].settings.auto_reconnect_secs = ar_secs;
                    app.worlds[idx].settings.auto_reconnect_on_web = ar_on_web;

                    // Update Slack settings
                    app.worlds[idx].settings.slack_token = settings.slack_token;
                    app.worlds[idx].settings.slack_channel = settings.slack_channel;
                    app.worlds[idx].settings.slack_workspace = settings.slack_workspace;

                    // Update Discord settings
                    app.worlds[idx].settings.discord_token = settings.discord_token;
                    app.worlds[idx].settings.discord_guild = settings.discord_guild;
                    app.worlds[idx].settings.discord_channel = settings.discord_channel;
                    app.worlds[idx].settings.discord_dm_user = settings.discord_dm_user;

                    app.add_output(&format!("World '{}' saved.", app.worlds[idx].name));
                    let _ = persistence::save_settings(app);
                }
            }
            NewPopupAction::WorldEditorDelete(idx) => {
                if app.worlds.len() > 1 && idx < app.worlds.len() {
                    let name = app.worlds[idx].name.clone();
                    app.open_delete_world_confirm(&name, idx);
                } else {
                    app.add_output("Cannot delete the last world.");
                }
            }
            NewPopupAction::WorldEditorConnect(idx) => {
                if idx < app.worlds.len() {
                    // First save the settings that were extracted
                    // Note: The settings were already saved via WorldEditorSaved before this
                    app.switch_world(idx);
                    if !app.current_world().connected {
                        return KeyAction::Connect;
                    }
                }
            }
            NewPopupAction::NotesList(action) => {
                match action {
                    NotesListAction::Open(name) => {
                        if let Some(idx) = app.find_world(&name) {
                            let notes = app.worlds[idx].settings.notes.clone();
                            app.editor.open_notes(idx, &notes);
                            app.needs_terminal_clear = true;
                        }
                    }
                }
            }
            NewPopupAction::None => {}
        }
        return KeyAction::None;
    }


    // Handle filter popup input
    if app.filter_popup.visible {
        match key.code {
            KeyCode::Esc => {
                app.filter_popup.close();
                app.needs_output_redraw = true;
            }
            KeyCode::F(4) => {
                // F4 again closes the filter
                app.filter_popup.close();
                app.needs_output_redraw = true;
            }
            KeyCode::F(2) => {
                // F2 toggles show_tags while filter is open
                app.show_tags = !app.show_tags;
                let output_lines = app.current_world().output_lines.clone();
                app.filter_popup.update_filter(&output_lines);
                app.needs_output_redraw = true;
            }
            KeyCode::Backspace => {
                if app.filter_popup.cursor > 0 {
                    app.filter_popup.cursor -= 1;
                    app.filter_popup.filter_text.remove(app.filter_popup.cursor);
                    let output_lines = app.current_world().output_lines.clone();
                    app.filter_popup.update_filter(&output_lines);
                    app.needs_output_redraw = true;
                }
            }
            KeyCode::Delete => {
                if app.filter_popup.cursor < app.filter_popup.filter_text.len() {
                    app.filter_popup.filter_text.remove(app.filter_popup.cursor);
                    let output_lines = app.current_world().output_lines.clone();
                    app.filter_popup.update_filter(&output_lines);
                    app.needs_output_redraw = true;
                }
            }
            KeyCode::Left | KeyCode::Char('b') if key.code == KeyCode::Left || key.modifiers.contains(KeyModifiers::CONTROL) => {
                // Left or Ctrl+B = cursor left
                if app.filter_popup.cursor > 0 {
                    app.filter_popup.cursor -= 1;
                }
            }
            KeyCode::Right | KeyCode::Char('f') if key.code == KeyCode::Right || key.modifiers.contains(KeyModifiers::CONTROL) => {
                // Right or Ctrl+F = cursor right
                if app.filter_popup.cursor < app.filter_popup.filter_text.len() {
                    app.filter_popup.cursor += 1;
                }
            }
            KeyCode::Home => {
                app.filter_popup.cursor = 0;
            }
            KeyCode::End => {
                app.filter_popup.cursor = app.filter_popup.filter_text.len();
            }
            KeyCode::PageUp => {
                // Scroll up in filtered results
                let visible_height = app.output_height as usize;
                app.filter_popup.scroll_offset = app.filter_popup.scroll_offset
                    .saturating_sub(visible_height.saturating_sub(2));
                app.needs_output_redraw = true;
            }
            KeyCode::PageDown => {
                // Scroll down in filtered results
                let visible_height = app.output_height as usize;
                let max_offset = app.filter_popup.filtered_indices.len().saturating_sub(1);
                app.filter_popup.scroll_offset = (app.filter_popup.scroll_offset + visible_height.saturating_sub(2))
                    .min(max_offset);
                app.needs_output_redraw = true;
            }
            KeyCode::Char(c) => {
                app.filter_popup.filter_text.insert(app.filter_popup.cursor, c);
                app.filter_popup.cursor += 1;
                let output_lines = app.current_world().output_lines.clone();
                app.filter_popup.update_filter(&output_lines);
                app.needs_output_redraw = true;
            }
            _ => {}
        }
        return KeyAction::None;
    }

    // Handle Tab - more-mode takes priority over command completion
    // Check more-mode first: scroll down when viewing history, release pending when at bottom and paused
    if key.code == KeyCode::Tab && key.modifiers.is_empty() {
        if !app.current_world().is_at_bottom() {
            // Viewing history (Hist indicator showing) - scroll down like PgDn
            app.scroll_output_down();
            app.needs_output_redraw = true;
            return KeyAction::None;
        } else if app.current_world().paused {
            // At bottom and paused - release one screenful (or just unpause if pending is empty)
            app.release_pending_screenful();
            return KeyAction::None;
        }
    }

    // Handle Tab for command completion when input starts with / (only if not in more-mode)
    let is_command_prefix = app.input.buffer.starts_with('/');
    if key.code == KeyCode::Tab && key.modifiers.is_empty() && is_command_prefix {
        // Get the current partial command (everything up to first space, or whole buffer)
        let input = app.input.buffer.clone();
        let partial = if let Some(space_pos) = input.find(' ') {
            &input[..space_pos]
        } else {
            input.as_str()
        };

        // Check if this is a /worlds or /world command with arguments (for world name completion)
        let is_worlds_cmd = partial.eq_ignore_ascii_case("/worlds") || partial.eq_ignore_ascii_case("/world");
        if is_worlds_cmd && input.contains(' ') {
            // World name completion for /worlds and /world commands
            let args_part = &input[input.find(' ').unwrap() + 1..];

            // Parse out -e or -l flag if present
            let (has_flag, partial_name) = if args_part.starts_with("-e ") || args_part.starts_with("-E ")
                || args_part.starts_with("-l ") || args_part.starts_with("-L ")
            {
                (true, args_part[3..].trim_start())
            } else if args_part == "-e" || args_part == "-E" || args_part == "-l" || args_part == "-L" {
                // Just the flag with no world name yet
                return KeyAction::None;
            } else {
                (false, args_part)
            };

            // Get matching world names
            let partial_lower = partial_name.to_lowercase();
            let mut world_matches: Vec<String> = app.worlds.iter()
                .map(|w| w.name.clone())
                .filter(|name| name.to_lowercase().starts_with(&partial_lower))
                .collect();
            world_matches.sort_by_key(|a| a.to_lowercase());

            if !world_matches.is_empty() {
                // Find current match index
                let current_idx = world_matches.iter().position(|m| m.eq_ignore_ascii_case(partial_name));
                let next_idx = match current_idx {
                    Some(idx) => (idx + 1) % world_matches.len(),
                    None => 0,
                };

                // Build the completed input
                let completion = &world_matches[next_idx];
                let flag_part = if has_flag {
                    if args_part.starts_with("-e") || args_part.starts_with("-E") {
                        "-e "
                    } else {
                        "-l "
                    }
                } else {
                    ""
                };
                app.input.buffer = format!("{} {}{}", partial, flag_part, completion);
                app.input.cursor_position = app.input.buffer.len();
            }
            return KeyAction::None;
        }

        // Only complete if we're still in the command part (no space yet or cursor before space)
        if !input.contains(' ') || app.input.cursor_position <= input.find(' ').unwrap_or(input.len()) {
            let matches = {
                // Unified / commands: Clay commands + TF commands + manual actions + macros
                let internal_commands = vec![
                    // Clay-specific commands
                    "/help", "/disconnect", "/dc", "/worlds", "/world", "/connections",
                    "/setup", "/web", "/actions", "/reload", "/update", "/quit", "/gag",
                    "/testmusic", "/dump", "/edit", "/tag", "/menu", "/notify",
                    // TF commands (now available with / prefix)
                    "/set", "/unset", "/let", "/echo", "/send", "/beep", "/quote",
                    "/expr", "/test", "/eval", "/if", "/elseif", "/else", "/endif",
                    "/while", "/done", "/for", "/break", "/def", "/undef", "/undefn",
                    "/undeft", "/list", "/purge", "/bind", "/unbind", "/load", "/save",
                    "/lcd", "/time", "/version", "/ps", "/kill", "/sh", "/recall",
                    "/setenv", "/listvar", "/repeat", "/fg", "/trigger", "/input",
                    "/grab", "/ungag", "/exit", "/addworld",
                    // TF-specific versions (for conflicting commands)
                    "/tfhelp", "/tfgag",
                ];

                // Get manual actions (empty pattern)
                let manual_actions: Vec<String> = app.settings.actions.iter()
                    .filter(|a| a.pattern.is_empty())
                    .map(|a| format!("/{}", a.name))
                    .collect();

                // Get user-defined macro names
                let macro_names: Vec<String> = app.tf_engine.macros.iter()
                    .map(|m| format!("/{}", m.name))
                    .collect();

                // Find all matches
                let partial_lower = partial.to_lowercase();
                let mut m: Vec<String> = internal_commands.iter()
                    .filter(|cmd| cmd.to_lowercase().starts_with(&partial_lower))
                    .map(|s| s.to_string())
                    .collect();
                m.extend(manual_actions.iter()
                    .filter(|cmd| cmd.to_lowercase().starts_with(&partial_lower))
                    .cloned());
                m.extend(macro_names.iter()
                    .filter(|cmd| cmd.to_lowercase().starts_with(&partial_lower))
                    .cloned());
                m.sort();
                m.dedup();
                m
            };

            if !matches.is_empty() {
                // Find current match index if we're already on a completed command
                let current_idx = matches.iter().position(|m| m.eq_ignore_ascii_case(partial));

                // Get next match (cycle through)
                let next_idx = match current_idx {
                    Some(idx) => (idx + 1) % matches.len(),
                    None => 0,
                };

                // Replace the command part with the completion
                let completion = &matches[next_idx];
                if input.contains(' ') {
                    // Preserve arguments after the command
                    let args_start = input.find(' ').unwrap();
                    app.input.buffer = format!("{}{}", completion, &input[args_start..]);
                } else {
                    app.input.buffer = completion.clone();
                }
                app.input.cursor_position = completion.len();
                return KeyAction::None;
            }
        }
    }

    // Ctrl+V literal next: insert next character literally
    if app.literal_next {
        app.literal_next = false;
        if let KeyCode::Char(c) = key.code {
            app.input.insert_char(c);
        }
        return KeyAction::None;
    }

    // Helper to check if escape was pressed recently (for Escape+key sequences)
    let recent_escape = app.last_escape
        .map(|t| t.elapsed() < Duration::from_millis(500))
        .unwrap_or(false);

    // Track bare Escape key presses for Escape+key sequences
    if key.code == KeyCode::Esc && key.modifiers.is_empty() {
        app.last_escape = Some(std::time::Instant::now());
        return KeyAction::None;
    }

    // Convert key event to canonical name, handling Esc+key sequences
    let key_name = if recent_escape && matches!(key.code, KeyCode::Char(_) | KeyCode::Backspace) {
        app.last_escape = None;
        keybindings::escape_key_to_name(key.code, key.modifiers)
    } else {
        keybindings::key_event_to_name(key.code, key.modifiers)
    };

    // Clear history search state on any non-search key
    if let Some(ref name) = key_name {
        if name != "Esc-p" && name != "Esc-n" && name != "Escape" {
            app.input.search_prefix = None;
            app.input.search_index = None;
        }
    }

    // Check TF /bind bindings first (runtime bindings from /bind command)
    if let Some(ref name) = key_name {
        // Map our canonical names to TF's key name format for lookup
        let tf_name = canonical_to_tf_key_name(name);
        if let Some(cmd) = app.tf_engine.keybindings.get(&tf_name).cloned() {
            return KeyAction::SendCommand(cmd);
        }
    }

    // Check configurable action bindings
    if let Some(ref name) = key_name {
        if let Some(action_id) = app.keybindings.get_action(name).map(|s| s.to_string()) {
            return dispatch_action(&action_id, app);
        }
    }

    // Enter key (not bound by default via action system - always active)
    if key.code == KeyCode::Enter {
        let input = app.input.take_input();
        if !input.is_empty() || app.current_world().connected {
            // /dump is passive — don't reset more-mode state
            let is_dump = input.trim().eq_ignore_ascii_case("/dump");
            if !is_dump {
                app.current_world_mut().lines_since_pause = 0;
                if app.current_world().pending_lines.is_empty() {
                    app.current_world_mut().paused = false;
                }
            }
            return KeyAction::SendCommand(input);
        }
        return KeyAction::None;
    }

    // Fall through to character input (unbound keys)
    if let KeyCode::Char(c) = key.code {
        if !key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
            if !c.is_alphabetic() && app.spell_state.showing_suggestions {
                app.spell_state.reset();
            }
            if !c.is_whitespace() && !matches!(c, '.' | ',' | '!' | '?' | ';' | ':' | ')' | ']' | '}') {
                app.skip_temp_conversion = None;
            }
            app.input.insert_char(c);
            app.last_input_was_delete = false;
            return KeyAction::None;
        }
    }

    KeyAction::None
}

/// Convert our canonical key names to TF's parse_key_name format for /bind lookup.
pub(crate) fn canonical_to_tf_key_name(name: &str) -> String {
    // Our format -> TF format:
    // "Esc-x" -> "Alt-X" (TF normalizes to Alt-)
    if let Some(rest) = name.strip_prefix("Esc-") {
        return format!("Alt-{}", rest.to_uppercase());
    }
    // "Ctrl-Up" -> needs to stay as-is since TF doesn't have these
    // "Shift-Up" -> same
    // "^A" -> "^A" (same format)
    // "F1" -> "F1" (same format)
    // Special keys like "PageUp", "Home", etc. match TF format
    name.to_string()
}

/// Dispatch a keybinding action ID to the corresponding behavior.
pub(crate) fn dispatch_action(action: &str, app: &mut App) -> KeyAction {
    match action {
        // Cursor Movement
        "cursor_left" => {
            app.input.move_cursor_left();
            KeyAction::None
        }
        "cursor_right" => {
            app.input.move_cursor_right();
            KeyAction::None
        }
        "cursor_word_left" => {
            app.input.word_left();
            KeyAction::None
        }
        "cursor_word_right" => {
            app.input.word_right();
            KeyAction::None
        }
        "cursor_home" => {
            app.input.home();
            KeyAction::None
        }
        "cursor_end" => {
            app.input.end();
            KeyAction::None
        }
        "cursor_up" => {
            if app.input.move_cursor_up() {
                app.input.history_prev();
            }
            KeyAction::None
        }
        "cursor_down" => {
            if app.input.move_cursor_down() {
                app.input.history_next();
            }
            KeyAction::None
        }

        // Editing
        "delete_backward" => {
            app.input.delete_char();
            app.last_input_was_delete = true;
            KeyAction::None
        }
        "delete_forward" => {
            app.input.delete_char_forward();
            app.last_input_was_delete = true;
            KeyAction::None
        }
        "delete_word_backward" => {
            app.input.delete_word_before_cursor();
            app.spell_state.reset();
            app.suggestion_message = None;
            app.last_input_was_delete = true;
            KeyAction::None
        }
        "delete_word_forward" => {
            app.input.delete_word_forward();
            app.last_input_was_delete = true;
            KeyAction::None
        }
        "delete_word_backward_punct" => {
            app.input.backward_kill_word_punctuation();
            app.last_input_was_delete = true;
            KeyAction::None
        }
        "kill_to_end" => {
            app.input.kill_to_end();
            app.last_input_was_delete = true;
            KeyAction::None
        }
        "clear_line" => {
            app.input.clear();
            app.spell_state.reset();
            app.suggestion_message = None;
            KeyAction::None
        }
        "transpose_chars" => {
            app.input.transpose_chars();
            KeyAction::None
        }
        "literal_next" => {
            app.literal_next = true;
            KeyAction::None
        }
        "capitalize_word" => {
            app.input.capitalize_word();
            KeyAction::None
        }
        "lowercase_word" => {
            app.input.lowercase_word();
            KeyAction::None
        }
        "uppercase_word" => {
            app.input.uppercase_word();
            KeyAction::None
        }
        "collapse_spaces" => {
            app.input.collapse_spaces();
            KeyAction::None
        }
        "goto_matching_bracket" => {
            app.input.goto_matching_bracket();
            KeyAction::None
        }
        "insert_last_arg" => {
            app.input.last_argument();
            KeyAction::None
        }
        "yank" => {
            app.input.yank();
            KeyAction::None
        }

        // History
        "history_prev" => {
            app.input.history_prev();
            app.spell_state.reset();
            KeyAction::None
        }
        "history_next" => {
            app.input.history_next();
            app.spell_state.reset();
            KeyAction::None
        }
        "history_search_backward" => {
            app.input.history_search_backward();
            KeyAction::None
        }
        "history_search_forward" => {
            app.input.history_search_forward();
            KeyAction::None
        }

        // Scrollback
        "scroll_page_up" => {
            app.scroll_output_up();
            KeyAction::None
        }
        "scroll_page_down" => {
            if app.current_world().is_at_bottom() && app.current_world().paused {
                app.release_pending_screenful();
            } else {
                app.scroll_output_down();
            }
            KeyAction::None
        }
        "scroll_half_page" => {
            let half = (app.output_height as usize).saturating_sub(2) / 2;
            if app.current_world().paused && !app.current_world().pending_lines.is_empty() {
                let visual_budget = half.max(1);
                let output_width = app.output_width as usize;
                let world_idx = app.current_world_index;
                let mut released = 0;
                let mut visual_used = 0;
                let pending_len = app.worlds[world_idx].pending_lines.len();
                for i in 0..pending_len {
                    let line = &app.worlds[world_idx].pending_lines[i];
                    let line_visual = if output_width > 0 {
                        (line.text.len() / output_width) + 1
                    } else {
                        1
                    };
                    if visual_used + line_visual > visual_budget && released > 0 {
                        break;
                    }
                    visual_used += line_visual;
                    released += 1;
                }
                if released > 0 {
                    let lines: Vec<_> = app.worlds[world_idx].pending_lines.drain(..released).collect();
                    // Group consecutive lines by marked_new for correct per-line indicators
                    let ts = current_timestamp_secs();
                    let mut batch: Vec<String> = Vec::new();
                    let mut batch_marked_new = lines.first().map(|l| l.marked_new).unwrap_or(false);
                    for line in &lines {
                        if line.marked_new != batch_marked_new && !batch.is_empty() {
                            let ws_data = batch.join("\n") + "\n";
                            app.ws_broadcast_to_world(world_idx, WsMessage::ServerData {
                                world_index: world_idx,
                                data: ws_data,
                                is_viewed: true,
                                ts,
                                from_server: true,
                                seq: 0,
                                marked_new: batch_marked_new,
                                flush: false, gagged: false,
                            });
                            batch.clear();
                            batch_marked_new = line.marked_new;
                        }
                        batch.push(line.text.replace('\r', ""));
                    }
                    if !batch.is_empty() {
                        let ws_data = batch.join("\n") + "\n";
                        app.ws_broadcast_to_world(world_idx, WsMessage::ServerData {
                            world_index: world_idx,
                            data: ws_data,
                            is_viewed: true,
                            ts,
                            from_server: true,
                            seq: 0,
                            marked_new: batch_marked_new,
                            flush: false, gagged: false,
                        });
                    }
                    app.worlds[world_idx].output_lines.extend(lines);
                    if app.worlds[world_idx].pending_lines.is_empty() {
                        app.worlds[world_idx].paused = false;
                        app.worlds[world_idx].lines_since_pause = 0;
                    }
                    let pending_count = app.worlds[world_idx].pending_lines.len();
                    app.ws_broadcast(WsMessage::PendingLinesUpdate { world_index: world_idx, count: pending_count });
                    app.broadcast_activity();
                }
            } else {
                app.scroll_output_up_by(half.max(1));
            }
            app.needs_output_redraw = true;
            KeyAction::None
        }
        "flush_output" => {
            if app.current_world().paused {
                let world_idx = app.current_world_index;
                let lines_with_flags: Vec<(String, bool)> = app.worlds[world_idx]
                    .pending_lines
                    .iter()
                    .map(|line| (line.text.replace('\r', ""), line.marked_new))
                    .collect();
                let released = lines_with_flags.len();
                app.current_world_mut().release_all_pending();
                if !lines_with_flags.is_empty() {
                    // Group consecutive lines by marked_new for correct per-line indicators
                    let ts = current_timestamp_secs();
                    let mut batch: Vec<&str> = Vec::new();
                    let mut batch_marked_new = lines_with_flags[0].1;
                    for (text, marked_new) in &lines_with_flags {
                        if *marked_new != batch_marked_new && !batch.is_empty() {
                            let ws_data = batch.join("\n") + "\n";
                            app.ws_broadcast_to_world(world_idx, WsMessage::ServerData {
                                world_index: world_idx,
                                data: ws_data,
                                is_viewed: true,
                                ts,
                                from_server: true,
                                seq: 0,
                                marked_new: batch_marked_new,
                                flush: false, gagged: false,
                            });
                            batch.clear();
                            batch_marked_new = *marked_new;
                        }
                        batch.push(text);
                    }
                    if !batch.is_empty() {
                        let ws_data = batch.join("\n") + "\n";
                        app.ws_broadcast_to_world(world_idx, WsMessage::ServerData {
                            world_index: world_idx,
                            data: ws_data,
                            is_viewed: true,
                            ts,
                            from_server: true,
                            seq: 0,
                            marked_new: batch_marked_new,
                            flush: false, gagged: false,
                        });
                    }
                }
                app.ws_broadcast(WsMessage::PendingReleased { world_index: world_idx, count: released });
                app.ws_broadcast(WsMessage::PendingLinesUpdate { world_index: world_idx, count: 0 });
                app.broadcast_activity();
                app.needs_output_redraw = true;
            }
            KeyAction::None
        }
        "selective_flush" => {
            if app.current_world().paused {
                let world_idx = app.current_world_index;
                let pending = std::mem::take(&mut app.worlds[world_idx].pending_lines);
                let mut kept = Vec::new();
                for line in pending {
                    if line.highlight_color.is_some() {
                        kept.push(line);
                    }
                }
                for line in &kept {
                    let ws_data = line.text.replace('\r', "") + "\n";
                    app.ws_broadcast_to_world(world_idx, WsMessage::ServerData {
                        world_index: world_idx,
                        data: ws_data,
                        is_viewed: true,
                        ts: current_timestamp_secs(),
                        from_server: line.from_server,
                        seq: line.seq,
                        marked_new: line.marked_new,
                        flush: false, gagged: false,
                    });
                }
                app.worlds[world_idx].output_lines.extend(kept);
                app.worlds[world_idx].paused = false;
                app.worlds[world_idx].lines_since_pause = 0;
                app.ws_broadcast(WsMessage::PendingLinesUpdate { world_index: world_idx, count: 0 });
                app.broadcast_activity();
                app.needs_output_redraw = true;
            }
            KeyAction::None
        }
        "tab_key" => {
            // Tab in more-mode releases a screenful, otherwise no-op for now
            // (Tab completion is handled earlier in the function)
            if app.current_world().paused {
                app.release_pending_screenful();
            } else if !app.current_world().is_at_bottom() {
                app.scroll_output_down();
            }
            KeyAction::None
        }

        // World
        "world_next" => {
            app.next_world();
            KeyAction::SwitchedWorld(app.current_world_index)
        }
        "world_prev" => {
            app.prev_world();
            KeyAction::SwitchedWorld(app.current_world_index)
        }
        "world_all_next" => {
            app.next_world();
            KeyAction::SwitchedWorld(app.current_world_index)
        }
        "world_all_prev" => {
            app.prev_world();
            KeyAction::SwitchedWorld(app.current_world_index)
        }
        "world_activity" => {
            app.switch_to_oldest_pending();
            KeyAction::None
        }
        "world_previous" => {
            app.prev_world();
            KeyAction::SwitchedWorld(app.current_world_index)
        }
        "world_forward" => {
            app.next_world();
            KeyAction::SwitchedWorld(app.current_world_index)
        }

        // System
        "help" => {
            app.open_help_popup_new();
            KeyAction::None
        }
        "redraw" => KeyAction::Redraw,
        "reload" => KeyAction::Reload,
        "quit" => {
            // Double-press Ctrl+C logic
            if let Some(last_time) = app.last_ctrl_c {
                if last_time.elapsed() < Duration::from_secs(15) {
                    return KeyAction::Quit;
                }
            }
            app.last_ctrl_c = Some(std::time::Instant::now());
            app.add_output("Press again within 15 seconds to exit, or use /quit");
            KeyAction::None
        }
        "suspend" => KeyAction::Suspend,
        "bell" => {
            print!("\x07");
            KeyAction::None
        }
        "spell_check" => {
            app.handle_spell_check();
            KeyAction::None
        }

        // Clay Extensions
        "toggle_tags" => {
            app.show_tags = !app.show_tags;
            app.current_world_mut().visual_line_offset = 0;
            KeyAction::Redraw
        }
        "filter_popup" => {
            app.filter_popup.open();
            let output_lines = app.current_world().output_lines.clone();
            app.filter_popup.update_filter(&output_lines);
            app.needs_output_redraw = true;
            KeyAction::None
        }
        "toggle_action_highlight" => {
            app.highlight_actions = !app.highlight_actions;
            KeyAction::Redraw
        }
        "toggle_gmcp_media" => {
            let idx = app.current_world_index;
            app.worlds[idx].gmcp_user_enabled = !app.worlds[idx].gmcp_user_enabled;
            app.ws_broadcast(WsMessage::GmcpUserToggled {
                world_index: idx,
                enabled: app.worlds[idx].gmcp_user_enabled,
            });
            if app.worlds[idx].gmcp_user_enabled {
                app.restart_world_media(idx);
            } else {
                app.stop_world_media(idx);
            }
            // Also toggle TTS mute
            app.settings.tts_muted = !app.settings.tts_muted;
            if app.settings.tts_muted {
                crate::tts::stop(&app.tts_backend);
            }
            app.needs_output_redraw = true;
            KeyAction::Redraw
        }
        "input_grow" => {
            app.increase_input_height();
            KeyAction::None
        }
        "input_shrink" => {
            app.decrease_input_height();
            KeyAction::None
        }

        _ => KeyAction::None,
    }
}
