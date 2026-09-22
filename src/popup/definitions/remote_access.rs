//! Remote Access popup — how other devices can reach this Clay.
//!
//! Opened from the /web popup's "Remote Access" button; `/reach` is its console
//! twin. Everything shown here is `reach::ReachabilityInfo` rendered through
//! `reach::format_reach_lines`, i.e. the same text `/reach` prints and the web/GUI
//! dialog shows — the popup never computes anything itself.
//!
//! The report is a *disabled* multiline field on purpose: a focused multiline
//! field is drawn with its cursor at the end of the text and scrolled to keep that
//! cursor visible, which would hide the LAN address (the line people need most).
//! Unfocusable, it renders from the top, so it is sized to hold the whole report.
//! The toggle and buttons act on `App` immediately; every result comes back via
//! `App::broadcast_reachability`, which calls [`update_report`] while this popup
//! is open.

use crate::popup::{
    Button, ButtonId, Field, FieldId, FieldKind, PopupDefinition, PopupId, PopupLayout, PopupState,
};
use crate::reach::{format_reach_lines, ReachabilityInfo};

pub const RA_FIELD_REPORT: FieldId = FieldId(1);
pub const RA_FIELD_PORTMAP: FieldId = FieldId(2);

pub const RA_BTN_FIREWALL: ButtonId = ButtonId(1);
pub const RA_BTN_LOOKUP: ButtonId = ButtonId(2);
pub const RA_BTN_REFRESH: ButtonId = ButtonId(3);
pub const RA_BTN_CLOSE: ButtonId = ButtonId(4);

/// Most rows the report may take: what fits on an 80x24 terminal once the popup
/// border, the toggle and the button row are accounted for.
const REPORT_MAX_LINES: usize = 17;
/// Wrap width used only to *estimate* how many display rows the report needs.
/// Deliberately the narrowest case — an 80-column terminal gives the popup 76
/// columns, minus the border and the 8-column "Status:" label — so the estimate
/// never undercounts; on a wider terminal a row or two at the bottom stay blank.
const REPORT_WRAP_ESTIMATE: usize = 62;

/// Build the Remote Access popup for the current reachability picture.
pub fn create_remote_access_popup(info: &ReachabilityInfo) -> PopupDefinition {
    let report = report_text(info);
    let visible = report_visible_lines(&report);
    let mut firewall_btn = Button::new(RA_BTN_FIREWALL, "Add Firewall Rule").with_shortcut('F');
    if !info.is_windows {
        firewall_btn = firewall_btn.disabled();
    }
    PopupDefinition::new(PopupId("remote_access"), "Remote Access")
        .with_field(
            Field::new(RA_FIELD_REPORT, "Status", FieldKind::multiline(report, visible)).disabled(),
        )
        .with_field(Field::new(
            RA_FIELD_PORTMAP,
            "Port Mapping (UPnP)",
            FieldKind::toggle(info.port_map_enabled),
        ))
        .with_button(firewall_btn)
        .with_button(Button::new(RA_BTN_LOOKUP, "Look Up Public IP").with_shortcut('L'))
        .with_button(Button::new(RA_BTN_REFRESH, "Refresh").with_shortcut('R'))
        .with_button(Button::new(RA_BTN_CLOSE, "Close").primary().with_shortcut('C'))
        .with_layout(PopupLayout {
            label_width: 8,
            min_width: 84,
            max_width_percent: 95,
            center_horizontal: true,
            center_vertical: true,
            modal: true,
            buttons_right_align: true,
            blank_line_before_list: false,
            tab_buttons_only: false,
            anchor_bottom_left: false,
            anchor_x: 0,
        })
        .with_help(remote_access_help_text())
}

/// Refresh the report and the toggle from a new picture (called by
/// `App::broadcast_reachability` whenever this popup is on top).
pub fn update_report(state: &mut PopupState, info: &ReachabilityInfo) {
    let report = report_text(info);
    let visible = report_visible_lines(&report);
    if let Some(field) = state.definition.get_field_mut(RA_FIELD_REPORT) {
        if let FieldKind::MultilineText { value, visible_lines, scroll_offset } = &mut field.kind {
            *value = report;
            *visible_lines = visible;
            *scroll_offset = 0;
        }
    }
    if let Some(field) = state.definition.get_field_mut(RA_FIELD_PORTMAP) {
        if let FieldKind::Toggle { value } = &mut field.kind {
            *value = info.port_map_enabled;
        }
    }
}

/// The `/reach` report minus its final "see the docs" line — the popup's own
/// help covers that, and every row counts on a small terminal.
fn report_text(info: &ReachabilityInfo) -> String {
    let mut lines = format_reach_lines(info);
    if lines.len() > 1 {
        lines.pop();
    }
    lines.join("\n")
}

/// Estimate the display rows the report needs (each line wrapped at
/// `REPORT_WRAP_ESTIMATE`), capped at `REPORT_MAX_LINES`.
fn report_visible_lines(report: &str) -> usize {
    let rows: usize = report
        .lines()
        .map(|l| l.chars().count().div_ceil(REPORT_WRAP_ESTIMATE).max(1))
        .sum();
    rows.clamp(4, REPORT_MAX_LINES)
}

/// Help text (F1 / ?) for the Remote Access popup.
pub fn remote_access_help_text() -> Vec<String> {
    [
        "Remote Access shows how other devices reach this Clay.",
        "/reach prints the same report in the output area.",
        "",
        "LAN: addresses on your local network. Other devices on",
        "  the same Wi-Fi/LAN use these directly.",
        "VPN: a 100.64.x.x address is Tailscale (or a similar",
        "  mesh VPN). Any device on that VPN can use it - no",
        "  port forwarding needed.",
        "Public IP: your internet address, reported by the",
        "  router (UPnP) or by Look Up Public IP, which asks",
        "  checkip.amazonaws.com and is never automatic. It is",
        "  only reachable with a router port mapping or a",
        "  manual port forward.",
        "",
        "Add Firewall Rule (Windows only): adds an inbound",
        "  Windows Defender Firewall rule for this program and",
        "  removes any Block rule left by clicking Cancel on the",
        "  first-run alert (a Block rule beats an Allow rule).",
        "  One UAC prompt, shown on this machine's own screen",
        "  even when the button is pressed from a phone. Applies",
        "  to all network profiles: home networks are often",
        "  classified Public, and the stealth path, password and",
        "  ban list are what actually gate access.",
        "",
        "Port Mapping (UPnP): asks the router to forward the web",
        "  port to this machine and keeps the lease renewed. Off",
        "  by default - it exposes the port to the whole internet",
        "  (see SECURITY-NOTES.md). If the router reports a",
        "  private WAN address it is itself behind another NAT",
        "  (carrier-grade NAT): use an SSH tunnel",
        "  (clay --gui=user@host --ssh) or a VPN instead.",
        "",
        "Refresh re-checks the firewall and the router mapping.",
        "See the Clay docs, Web Interface -> Reaching Clay from",
        "outside your network.",
    ]
    .into_iter()
    .map(|s| s.to_string())
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reach::{FirewallStatus, PortMapStatus};

    fn info() -> ReachabilityInfo {
        let mut info = ReachabilityInfo {
            http_enabled: true,
            port: 9000,
            web_path: "clay".into(),
            lan_ips: vec!["192.168.1.20".into()],
            lan_urls: vec!["https://192.168.1.20:9000/clay/".into()],
            firewall: FirewallStatus::NotApplicable,
            is_windows: false,
            ..Default::default()
        };
        info.firewall_text = crate::reach::firewall_text(&info.firewall);
        info.port_map_text = crate::reach::port_map_text(&info.port_map, false);
        info.client_hints = crate::reach::build_client_hints(&info);
        info
    }

    #[test]
    fn test_remote_access_popup_creation() {
        let def = create_remote_access_popup(&info());
        let state = PopupState::new(def);
        assert_eq!(state.definition.id, PopupId("remote_access"));
        assert_eq!(state.definition.title, "Remote Access");
        assert_eq!(state.definition.fields.len(), 2);
        // Four action buttons (plus the [?] help button that with_help() adds).
        for id in [RA_BTN_FIREWALL, RA_BTN_LOOKUP, RA_BTN_REFRESH, RA_BTN_CLOSE] {
            assert!(state.definition.buttons.iter().any(|b| b.id == id), "missing button {id:?}");
        }
        let report = state.get_text(RA_FIELD_REPORT).unwrap();
        assert!(report.starts_with("Remote access to this Clay"));
        assert!(report.contains("https://192.168.1.20:9000/clay/"));
        assert!(!report.contains("See the Clay docs"), "docs pointer belongs to the help text");
        assert_eq!(state.get_bool(RA_FIELD_PORTMAP), Some(false));
        // The report is display-only: never focusable, so it never auto-scrolls to its end.
        let report_field = state.definition.get_field(RA_FIELD_REPORT).unwrap();
        assert!(!report_field.is_focusable());
        // Off Windows the firewall button is present but disabled.
        assert!(!state.definition.buttons.iter().find(|b| b.id == RA_BTN_FIREWALL).unwrap().enabled);
    }

    #[test]
    fn test_update_report_refreshes_text_and_toggle() {
        let mut state = PopupState::new(create_remote_access_popup(&info()));
        let mut newer = info();
        newer.port_map_enabled = true;
        newer.port_map = PortMapStatus::NoGateway;
        newer.port_map_text = crate::reach::port_map_text(&newer.port_map, true);
        update_report(&mut state, &newer);
        assert!(state.get_text(RA_FIELD_REPORT).unwrap().contains("no UPnP gateway answered"));
        assert_eq!(state.get_bool(RA_FIELD_PORTMAP), Some(true));
    }

    #[test]
    fn test_report_visible_lines_bounds() {
        assert_eq!(report_visible_lines(""), 4);
        let long = "x".repeat(150);
        assert_eq!(report_visible_lines(&format!("{long}\n{long}\n{long}")), 9);
        let many: Vec<String> = (0..40).map(|i| format!("line {i}")).collect();
        assert_eq!(report_visible_lines(&many.join("\n")), REPORT_MAX_LINES);
    }
}
