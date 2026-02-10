//! Connections popup definition
//!
//! Displays the list of connected worlds with status information.

use crate::popup::{
    Button, ButtonId, Field, FieldId, FieldKind, ListItem, ListItemStyle,
    PopupDefinition, PopupId, PopupLayout,
};

// Field IDs
pub const CONNECTIONS_FIELD_LIST: FieldId = FieldId(1);

// Button IDs
pub const CONNECTIONS_BTN_OK: ButtonId = ButtonId(1);

/// Information about a connection for display
#[derive(Debug, Clone)]
pub struct ConnectionInfo {
    pub name: String,
    pub is_current: bool,
    pub is_connected: bool,
    pub is_ssl: bool,
    pub is_proxy: bool,
    pub unseen_lines: usize,
    pub last: String,
    pub ka: String,
    pub buffer_size: usize,
}

/// Format elapsed time in a human-readable format
pub fn format_elapsed(secs: Option<u64>) -> String {
    match secs {
        None => "-".to_string(),
        Some(s) => {
            if s < 60 {
                format!("{}s", s)
            } else if s < 3600 {
                format!("{}m", s / 60)
            } else if s < 86400 {
                format!("{}h", s / 3600)
            } else {
                format!("{}d", s / 86400)
            }
        }
    }
}

/// Format time until next NOP
pub fn format_next_nop(last_send_secs: Option<u64>, last_recv_secs: Option<u64>) -> String {
    const KEEPALIVE_SECS: u64 = 5 * 60;
    let elapsed = match (last_send_secs, last_recv_secs) {
        (Some(s), Some(r)) => s.min(r),
        (Some(s), None) => s,
        (None, Some(r)) => r,
        (None, None) => KEEPALIVE_SECS,
    };
    let remaining = KEEPALIVE_SECS.saturating_sub(elapsed);
    if remaining < 60 {
        format!("{}s", remaining)
    } else {
        format!("{}m", remaining / 60)
    }
}

/// Create the connections popup definition
pub fn create_connections_popup(connections: &[ConnectionInfo], visible_height: usize) -> PopupDefinition {
    let items: Vec<ListItem> = connections
        .iter()
        .filter(|c| c.is_connected)
        .map(|c| {
            // SSL/Proxy indicator
            let ssl = if c.is_ssl {
                if c.is_proxy { "PRX" } else { "SSH" }
            } else {
                ""
            };

            // Current world indicator
            let current = if c.is_current { "*" } else { " " };

            // Unseen count
            let unseen = if c.unseen_lines > 0 {
                c.unseen_lines.to_string()
            } else {
                String::new()
            };

            let columns = vec![
                format!("{}{:3}", current, ssl),
                c.name.clone(),
                unseen,
                c.last.clone(),
                c.ka.clone(),
                c.buffer_size.to_string(),
            ];

            ListItem {
                id: c.name.clone(),
                columns,
                style: ListItemStyle {
                    is_current: c.is_current,
                    is_connected: c.is_connected,
                    ..Default::default()
                },
            }
        })
        .collect();

    let is_empty = items.is_empty();

    PopupDefinition::new(PopupId("connections"), "Connected Worlds")
        .with_field(if is_empty {
            Field::new(
                CONNECTIONS_FIELD_LIST,
                "",
                FieldKind::label("No worlds connected."),
            )
        } else {
            Field::new(
                CONNECTIONS_FIELD_LIST,
                "",
                FieldKind::list_with_headers_and_widths(
                    items,
                    visible_height,
                    &["", "World", "Unseen", "Last", "KA", "Buffer"],
                    vec![4, 20, 6, 9, 9, 7],
                ),
            )
        })
        .with_button(Button::new(CONNECTIONS_BTN_OK, "OK").primary().with_shortcut('O'))
        .with_layout(PopupLayout {
            label_width: 0,
            min_width: 50,
            max_width_percent: 70,
            center_horizontal: true,
            center_vertical: true,
            modal: true,
            buttons_right_align: false,
            blank_line_before_list: false,
            tab_buttons_only: false,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::popup::PopupState;

    fn sample_connections() -> Vec<ConnectionInfo> {
        vec![
            ConnectionInfo {
                name: "TestMUD".to_string(),
                is_current: true,
                is_connected: true,
                is_ssl: true,
                is_proxy: false,
                unseen_lines: 0,
                last: "2s/5s".to_string(),
                ka: "4m/4m".to_string(),
                buffer_size: 1500,
            },
            ConnectionInfo {
                name: "AnotherWorld".to_string(),
                is_current: false,
                is_connected: true,
                is_ssl: false,
                is_proxy: false,
                unseen_lines: 15,
                last: "30s/1m".to_string(),
                ka: "3m/4m".to_string(),
                buffer_size: 3200,
            },
        ]
    }

    #[test]
    fn test_connections_popup_creation() {
        let connections = sample_connections();
        let def = create_connections_popup(&connections, 10);
        let state = PopupState::new(def);

        assert_eq!(state.definition.id, PopupId("connections"));
        assert_eq!(state.definition.title, "Connected Worlds");
    }

    #[test]
    fn test_empty_connections() {
        let def = create_connections_popup(&[], 10);
        let state = PopupState::new(def);

        assert_eq!(state.definition.id, PopupId("connections"));
        // Should have a label field instead of list
        if let Some(field) = state.field(CONNECTIONS_FIELD_LIST) {
            assert!(matches!(&field.kind, FieldKind::Label { .. }));
        }
    }

    #[test]
    fn test_format_elapsed() {
        assert_eq!(format_elapsed(None), "-");
        assert_eq!(format_elapsed(Some(30)), "30s");
        assert_eq!(format_elapsed(Some(90)), "1m");
        assert_eq!(format_elapsed(Some(3700)), "1h");
        assert_eq!(format_elapsed(Some(90000)), "1d");
    }
}
