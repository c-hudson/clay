//! The Web settings' two dialogs:
//! - Modify Key (app.js `showModifyKeyDialog`): copy, regenerate or delete the auth key.
//! - Remote Access (`showRemoteAccessDialog`): how other devices reach this Clay, from
//!   the server's own `reach::ReachabilityInfo`, shown as it is and never recomputed.

use gpui::{
    App, ClipboardItem, Context, EventEmitter, FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, Styled, Subscription, WeakEntity, Window, div, prelude::*, px, relative,
};

use super::DialogEvent;
use crate::native_gui::ui::ClayView;
use crate::native_gui::ui::kit::{Btn, Click, Kit};
use crate::native_gui::ui::windows::MainEvent;
use crate::reach::{FirewallStatus, PortMapStatus, ReachabilityInfo};
use crate::websocket::WsMessage;

fn kit(main: &WeakEntity<ClayView>, cx: &App) -> Option<Kit> {
    main.upgrade().map(|m| m.read(cx).kit())
}

fn settings<T>(main: &WeakEntity<ClayView>, cx: &App, f: impl FnOnce(&crate::websocket::GlobalSettingsMsg) -> T) -> Option<T> {
    main.upgrade().and_then(|m| m.read(cx).session.core.settings.as_ref().map(f))
}

/// Redraw when the main view's settings change.
fn follow(main: &WeakEntity<ClayView>, cx: &mut Context<impl EventEmitter<DialogEvent> + 'static>) -> Option<Subscription> {
    main.upgrade().map(|m| cx.subscribe(&m, |_, _, _: &MainEvent, cx| cx.notify()))
}

pub struct ModifyKey {
    main: WeakEntity<ClayView>,
    focus: FocusHandle,
    _follow: Option<Subscription>,
}

impl ModifyKey {
    pub fn new(main: WeakEntity<ClayView>, cx: &mut Context<Self>) -> Self {
        let follow = follow(&main, cx);
        Self { main, focus: cx.focus_handle(), _follow: follow }
    }
}

impl EventEmitter<DialogEvent> for ModifyKey {}
impl Focusable for ModifyKey {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ModifyKey {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(k) = kit(&self.main, cx) else { return div().into_any_element() };
        let key = settings(&self.main, cx, |s| s.auth_key.clone()).unwrap_or_default();
        let (copy, revoke) = (key.clone(), key.clone());
        let main = self.main.clone();
        k.panel("modify-key")
            .track_focus(&self.focus)
            .max_w(px(480.0))
            .on_key_down(cx.listener(|_, ev: &gpui::KeyDownEvent, _w, cx| {
                if ev.keystroke.key == "escape" {
                    cx.emit(DialogEvent::Close);
                    cx.stop_propagation();
                }
            }))
            .child(div().text_size(px(16.0)).font_weight(FontWeight::BOLD).child("Modify Auth Key"))
            .child(k.mono(if key.is_empty() { "(none)".to_string() } else { key.clone() }).opacity(0.9))
            .child(
                k.buttons()
                    .flex_wrap()
                    .child(k.button("modify-key-close", "Close", Btn::Plain, Some(Box::new(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))))))
                    .child(k.button(
                        "modify-key-delete",
                        "Delete",
                        Btn::Danger,
                        Some(Box::new(cx.listener(move |_, _, _, cx| {
                            let auth_key = revoke.clone();
                            let _ = main.update(cx, |v, cx| {
                                v.send_message(WsMessage::RevokeKey { auth_key }, cx);
                                v.session.core.forget_auth_key();
                                cx.notify();
                            });
                            cx.emit(DialogEvent::Close);
                        }))),
                    ))
                    .child(k.button(
                        "modify-key-regen",
                        "Regen",
                        Btn::Plain,
                        // The new key arrives as KeyGenerated, and this redraws with it.
                        Some(Box::new(cx.listener(|this, _, _, cx| {
                            let _ = this.main.update(cx, |v, cx| v.send_message(WsMessage::RegenerateAuthKey, cx));
                        }))),
                    ))
                    .child(k.button(
                        "modify-key-copy",
                        "Copy",
                        Btn::Plain,
                        (!copy.is_empty()).then(|| -> Click { Box::new(move |_, _, cx: &mut App| cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))) }),
                    )),
            )
            .into_any_element()
    }
}

pub struct RemoteAccess {
    main: WeakEntity<ClayView>,
    focus: FocusHandle,
    status: String,
    _follow: Option<Subscription>,
}

impl RemoteAccess {
    pub fn new(main: WeakEntity<ClayView>, cx: &mut Context<Self>) -> Self {
        let follow = follow(&main, cx);
        // Ask the server to look at the firewall and router again; its answer redraws this.
        let _ = main.update(cx, |v, cx| v.send_message(WsMessage::RefreshReachability, cx));
        Self { main, focus: cx.focus_handle(), status: String::new(), _follow: follow }
    }

    fn send(&mut self, msg: WsMessage, status: &str, cx: &mut Context<Self>) {
        self.status = status.to_string();
        let _ = self.main.update(cx, |v, cx| v.send_message(msg, cx));
        cx.notify();
    }
}

impl EventEmitter<DialogEvent> for RemoteAccess {}
impl Focusable for RemoteAccess {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// The report's rows (app.js `renderRemoteAccessDialog`): (key, value, good/bad/plain).
pub fn report(info: Option<&ReachabilityInfo>) -> Vec<(&'static str, String, Option<bool>)> {
    let Some(info) = info else { return vec![("Status", "waiting for the server...".into(), None)] };
    let mut rows = Vec::new();
    let path = if info.web_path.is_empty() { "/".to_string() } else { format!("/{}/", info.web_path) };
    rows.push((
        "Web server",
        if info.http_enabled { format!("on, port {}, path {path}", info.port) } else { "OFF (enable it in Web Settings)".into() },
        (!info.http_enabled).then_some(false),
    ));
    rows.push(("LAN", if info.lan_urls.is_empty() { "no non-loopback IPv4 address found".into() } else { info.lan_urls.join("\n") }, None));
    if !info.vpn_addrs.is_empty() {
        rows.push(("VPN", format!("{}  (Tailscale or similar; reachable from that VPN without port forwarding)", info.vpn_addrs.join(", ")), None));
    }
    let public = match (&info.public_ip, &info.public_ip_error) {
        (Some(ip), _) => format!(
            "{ip}{}",
            match info.public_ip_source.as_str() {
                "upnp" => " (reported by the router)",
                "lookup" => " (looked up)",
                _ => "",
            }
        ),
        (None, Some(e)) => format!("lookup failed: {e}"),
        (None, None) => "unknown (use Look Up Public IP)".into(),
    };
    rows.push(("Public IP", public, None));
    let fw = match info.firewall {
        FirewallStatus::Blocked | FirewallStatus::Missing => Some(false),
        FirewallStatus::Allowed { .. } => Some(true),
        _ => None,
    };
    rows.push(("Firewall", info.firewall_text.clone(), fw));
    let pm = match info.port_map {
        PortMapStatus::Mapped { .. } => Some(true),
        PortMapStatus::DoubleNat { .. } | PortMapStatus::Failed { .. } | PortMapStatus::NoGateway => Some(false),
        _ => None,
    };
    rows.push(("Router", info.port_map_text.clone(), pm));
    rows
}

impl Render for RemoteAccess {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(k) = kit(&self.main, cx) else { return div().into_any_element() };
        let t = k.t.clone();
        let info: Option<ReachabilityInfo> = settings(&self.main, cx, |s| serde_json::from_str(&s.reachability_json).ok()).flatten();
        let row = |key: &str, value: String, tone: Option<bool>| {
            let color = match tone {
                Some(true) => t.success,
                Some(false) => t.error,
                None => t.fg,
            };
            div()
                .flex()
                .gap(px(10.0))
                .py(px(2.0))
                .child(div().w(px(90.0)).flex_shrink_0().text_color(t.fg2).child(key.to_string()))
                .child(div().flex_1().min_w_0().text_color(color).children(value.lines().map(|l| div().child(l.to_string())).collect::<Vec<_>>()))
        };
        let rows: Vec<_> = report(info.as_ref()).into_iter().map(|(k, v, tone)| row(k, v, tone)).collect();
        let hints: Vec<_> = info
            .as_ref()
            .map(|i| i.client_hints.clone())
            .unwrap_or_default()
            .into_iter()
            .enumerate()
            .map(|(n, h)| {
                let value = h.value.clone();
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .py(px(2.0))
                    .child(div().w(px(90.0)).flex_shrink_0().text_color(t.fg2).child(h.label.clone()))
                    .child(div().flex_1().min_w_0().font_family(k.fonts.primary.clone()).text_size(px(12.0)).child(h.value.clone()))
                    .child(
                        k.button(("reach-copy", n), "Copy", Btn::Plain, Some(Box::new(move |_, _, cx: &mut App| cx.write_to_clipboard(ClipboardItem::new_string(value.clone())))))
                            .px(px(8.0))
                            .py(px(2.0)),
                    )
            })
            .collect();
        let is_windows = info.as_ref().is_some_and(|i| i.is_windows);
        let mapped = info.as_ref().is_some_and(|i| i.port_map_enabled);
        k.panel("remote-access")
            .track_focus(&self.focus)
            .w(px(620.0))
            .max_w(window.viewport_size().width * 0.96)
            .max_h(window.viewport_size().height * 0.9)
            .on_key_down(cx.listener(|_, ev: &gpui::KeyDownEvent, _w, cx| {
                if ev.keystroke.key == "escape" {
                    cx.emit(DialogEvent::Close);
                    cx.stop_propagation();
                }
            }))
            .child(div().text_size(px(16.0)).font_weight(FontWeight::BOLD).child("Remote Access"))
            .child(k.note("How other devices reach this Clay. Same as typing /reach."))
            .child(
                div()
                    .id("reach-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .line_height(relative(1.4))
                    .child(div().flex().flex_col().children(rows))
                    .child(div().font_weight(FontWeight::BOLD).mt(px(6.0)).child("Connect from another device"))
                    .child(div().flex().flex_col().children(hints))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .mt(px(8.0))
                            .child(k.toggle("reach-portmap", mapped, cx.listener(move |this, _, _w, cx| this.send(WsMessage::SetPortMapping { enabled: !mapped }, "", cx))))
                            .child("Port Mapping (UPnP): ask the router to forward the port to this machine"),
                    )
                    .child(div().min_h(px(17.0)).opacity(0.85).child(SharedString::from(self.status.clone()))),
            )
            .child(
                k.buttons()
                    .flex_wrap()
                    .child(k.button("reach-close", "Close", Btn::Plain, Some(Box::new(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))))))
                    .child(k.button(
                        "reach-refresh",
                        "Refresh",
                        Btn::Plain,
                        Some(Box::new(cx.listener(|this, _, _, cx| this.send(WsMessage::RefreshReachability, "Re-checking firewall and router...", cx)))),
                    ))
                    .child(k.button(
                        "reach-lookup",
                        "Look Up Public IP",
                        Btn::Plain,
                        Some(Box::new(cx.listener(|this, _, _, cx| this.send(WsMessage::LookupPublicIp, "Looking up public IP (asks checkip.amazonaws.com)...", cx)))),
                    ))
                    // Windows only: one UAC prompt on the Clay machine.
                    .child(k.button(
                        "reach-firewall",
                        "Add Firewall Rule",
                        Btn::Plain,
                        is_windows.then(|| -> Click {
                            Box::new(cx.listener(|this, _, _, cx| {
                                this.send(WsMessage::FirewallRule { action: "add".into() }, "Answer the UAC prompt on the Clay machine's own screen...", cx)
                            }))
                        }),
                    )),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_report_reads_the_servers_picture() {
        assert_eq!(report(None)[0].1, "waiting for the server...");
        let info = ReachabilityInfo { http_enabled: true, port: 9000, web_path: "clay".into(), ..Default::default() };
        let rows = report(Some(&info));
        assert_eq!(rows[0], ("Web server", "on, port 9000, path /clay/".to_string(), None));
        assert!(rows.iter().any(|r| r.0 == "Public IP" && r.1 == "unknown (use Look Up Public IP)"));
    }
}
