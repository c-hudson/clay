//! The small dialogs:
//! - the connection log (app.js `showConnectionLog`)
//! - Connection Lost (`showReconnectModal`)
//! - Change Password (`showPasswordModal`)
//! - /menu (`openMenuPopup`)
//! - a dialog's "?" help (`openPopupHelp`)
//! - a MUD world's changed certificate (`showCertMismatchDialog`)
//! - /import and its plaintext warning (`showImportDialog`, `showImportInsecureConfirmDialog`)
//! - Recent Worlds (`showRecentWorldsModal`; the console's recent-worlds popup)
//! - Custom Text Reveal Speed (`showRevealSpeedDialog`; the console's reveal_speed popup)

use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, WeakEntity, Window, div, prelude::*, px, relative,
};
use gpui_component::input::{InputEvent, InputState};

use super::DialogEvent;
use crate::native_gui::ui::ClayView;
use crate::native_gui::ui::kit::{Btn, Click, Kit};
use crate::websocket::WsMessage;

fn kit(main: &WeakEntity<ClayView>, cx: &App) -> Option<Kit> {
    main.upgrade().map(|m| m.read(cx).kit())
}

/// Esc closes (and Enter does `enter`, when given).
fn keys<V: EventEmitter<DialogEvent>>(enter: Option<fn(&mut V, &mut Window, &mut Context<V>)>) -> impl Fn(&mut V, &KeyDownEvent, &mut Window, &mut Context<V>) {
    move |this, ev, window, cx| match ev.keystroke.key.as_str() {
        "escape" => {
            cx.emit(DialogEvent::Close);
            cx.stop_propagation();
        }
        "enter" => {
            if let Some(f) = enter {
                f(this, window, cx);
                cx.stop_propagation();
            }
        }
        _ => {}
    }
}

// ---- The connection log -------------------------------------------------------------

/// One try at connecting, as the log lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct Attempt {
    pub url: String,
    /// `None` while it's going; then whether it worked.
    pub ok: Option<bool>,
    pub note: Option<String>,
}

/// The attempts since the last time it was online, and whether Retry is offered (after
/// app.js's five failures in a row).
#[derive(Default)]
pub struct ConnLog {
    pub attempts: Vec<Attempt>,
    pub retry: bool,
    pub failures: u32,
}

pub struct ConnectionLog {
    main: WeakEntity<ClayView>,
    focus: FocusHandle,
}

impl ConnectionLog {
    pub fn new(main: WeakEntity<ClayView>, cx: &mut Context<Self>) -> Self {
        Self { main, focus: cx.focus_handle() }
    }
}

impl EventEmitter<DialogEvent> for ConnectionLog {}
impl Focusable for ConnectionLog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ConnectionLog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(k) = kit(&self.main, cx) else { return div().into_any_element() };
        let (attempts, retry) = self.main.upgrade().map(|m| {
            let v = m.read(cx);
            (v.surf.conn_log.attempts.clone(), v.surf.conn_log.retry)
        }).unwrap_or_default();
        let rows = attempts.into_iter().enumerate().map(|(i, a)| {
            let (icon, color) = match a.ok {
                None => ("⟳", k.t.fg),
                Some(true) => ("✓", k.t.success),
                Some(false) => ("✗", k.t.error),
            };
            let text = match a.note {
                Some(n) => format!("{} {n}", a.url),
                None => a.url,
            };
            div()
                .id(("attempt", i))
                .flex()
                .items_center()
                .gap(px(10.0))
                .py(px(5.0))
                .border_b_1()
                .border_color(k.t.border_subtle)
                .child(div().w(px(16.0)).flex_shrink_0().text_color(color).child(icon))
                .child(div().min_w_0().opacity(0.85).child(text))
        });
        let main = self.main.clone();
        let main2 = self.main.clone();
        k.panel("connection-log")
            .track_focus(&self.focus)
            .max_w(px(560.0))
            .child(k.heading("Connecting to Clay Server"))
            .child(
                div()
                    .id("connection-log-list")
                    .flex()
                    .flex_col()
                    .max_h(px(220.0))
                    .overflow_y_scroll()
                    .font_family(k.fonts.primary.clone())
                    .text_size(px(13.0))
                    .children(rows),
            )
            .child(
                k.buttons()
                    .child(k.button(
                        "connection-log-retry",
                        "Retry",
                        Btn::Plain,
                        retry.then(|| -> Click {
                            Box::new(move |_, _, cx| {
                                let _ = main.update(cx, |v, cx| v.reconnect_now(cx));
                            })
                        }),
                    ))
                    .child(k.button(
                        "connection-log-settings",
                        "Settings",
                        Btn::Plain,
                        Some(Box::new(cx.listener(move |_, _, window, cx| {
                            cx.emit(DialogEvent::Close);
                            let _ = main2.update(cx, |v, cx| v.open_surface("setup", window, cx));
                        }))),
                    )),
            )
            .into_any_element()
    }
}

// ---- Connection Lost ----------------------------------------------------------------

/// A command couldn't go: offer to reconnect, and send it once back online.
pub struct Reconnect {
    main: WeakEntity<ClayView>,
    focus: FocusHandle,
}

impl Reconnect {
    pub fn new(main: WeakEntity<ClayView>, cx: &mut Context<Self>) -> Self {
        Self { main, focus: cx.focus_handle() }
    }

    fn reconnect(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(DialogEvent::Close);
        let _ = self.main.update(cx, |v, cx| v.reconnect_now(cx));
    }

    fn cancel(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(DialogEvent::Close);
        let _ = self.main.update(cx, |v, _| v.surf.pending_command = None);
    }
}

impl EventEmitter<DialogEvent> for Reconnect {}
impl Focusable for Reconnect {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Reconnect {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(k) = kit(&self.main, cx) else { return div().into_any_element() };
        k.panel("reconnect")
            .track_focus(&self.focus)
            .max_w(px(420.0))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| match ev.keystroke.key.as_str() {
                "escape" => this.cancel(window, cx),
                "enter" => this.reconnect(window, cx),
                _ => {}
            }))
            .child(k.heading("Connection Lost"))
            .child(k.text("Unable to send command. The connection to the server was lost."))
            .child(
                k.buttons()
                    .child(k.button("reconnect-cancel", "Cancel", Btn::Plain, Some(Box::new(cx.listener(|this, _, w, cx| this.cancel(w, cx))))))
                    .child(k.button("reconnect-go", "Reconnect", Btn::Primary, Some(Box::new(cx.listener(|this, _, w, cx| this.reconnect(w, cx)))))),
            )
            .into_any_element()
    }
}

// ---- Change Password ----------------------------------------------------------------

/// Multiuser: change this user's password (`ChangePassword`; the answer is
/// `PasswordChanged`, which closes this or shows its error).
pub struct Password {
    main: WeakEntity<ClayView>,
    old: Entity<InputState>,
    new: Entity<InputState>,
    confirm: Entity<InputState>,
    pub error: Option<String>,
}

impl Password {
    pub fn new(main: WeakEntity<ClayView>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let field = |window: &mut Window, cx: &mut Context<Self>| cx.new(|cx| InputState::new(window, cx).masked(true));
        let (old, new, confirm) = (field(window, cx), field(window, cx), field(window, cx));
        // Enter moves to the next field; on the last, it saves.
        for (from, to) in [(&old, Some(new.clone())), (&new, Some(confirm.clone())), (&confirm, None)] {
            cx.subscribe_in(from, window, move |this, _, ev: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = ev {
                    match &to {
                        Some(next) => next.update(cx, |f, cx| f.focus(window, cx)),
                        None => this.save(window, cx),
                    }
                }
            })
            .detach();
        }
        Self { main, old, new, confirm, error: None }
    }

    fn save(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let old = self.old.read(cx).value().to_string();
        let new = self.new.read(cx).value().to_string();
        let confirm = self.confirm.read(cx).value().to_string();
        self.error = password_problem(&old, &new, &confirm).map(str::to_string);
        if self.error.is_none() {
            let msg = WsMessage::ChangePassword {
                old_password_hash: crate::websocket::hash_password(&old),
                new_password_hash: crate::websocket::hash_password(&new),
            };
            let _ = self.main.update(cx, |v, cx| v.send_message(msg, cx));
        }
        cx.notify();
    }
}

/// app.js's checks before sending, in its order.
pub fn password_problem(old: &str, new: &str, confirm: &str) -> Option<&'static str> {
    if old.is_empty() || new.is_empty() || confirm.is_empty() {
        Some("All fields are required")
    } else if new != confirm {
        Some("New passwords do not match")
    } else if new.chars().count() < 4 {
        Some("New password must be at least 4 characters")
    } else {
        None
    }
}

impl EventEmitter<DialogEvent> for Password {}
impl Focusable for Password {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.old.read(cx).focus_handle(cx)
    }
}

impl Render for Password {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(k) = kit(&self.main, cx) else { return div().into_any_element() };
        k.panel("password")
            .w(px(340.0))
            .on_key_down(cx.listener(keys::<Self>(None)))
            .child(k.heading("Change Password"))
            .child(k.field("Current Password:", &self.old))
            .child(k.field("New Password:", &self.new))
            .child(k.field("Confirm Password:", &self.confirm))
            .child(k.error(self.error.as_deref()))
            .child(
                k.buttons()
                    .child(k.button("password-cancel", "Cancel", Btn::Plain, Some(Box::new(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))))))
                    .child(k.button("password-save", "Change Password", Btn::Primary, Some(Box::new(cx.listener(|this, _, w, cx| this.save(w, cx)))))),
            )
            .into_any_element()
    }
}

// ---- A yes/no question --------------------------------------------------------------

/// What Yes does.
pub type OnYes = Box<dyn Fn(&mut Window, &mut App)>;

/// "Delete action 'x'?" and the like (index.html `#action-confirm-modal`): Yes runs the
/// deed, No and Esc don't.
pub struct Confirm {
    main: WeakEntity<ClayView>,
    focus: FocusHandle,
    text: String,
    yes: OnYes,
    /// Y or Enter for Yes, N for No (the world confirm's keys; the action one has only Esc).
    pub yn_keys: bool,
}

impl Confirm {
    pub fn new(main: WeakEntity<ClayView>, text: String, yes: OnYes, cx: &mut Context<Self>) -> Self {
        Self { main, focus: cx.focus_handle(), text, yes, yn_keys: false }
    }
}

impl EventEmitter<DialogEvent> for Confirm {}
impl Focusable for Confirm {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Confirm {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(k) = kit(&self.main, cx) else { return div().into_any_element() };
        k.panel("confirm")
            .track_focus(&self.focus)
            .max_w(px(420.0))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                let key = ev.keystroke.key.as_str();
                let yes = this.yn_keys && (key == "enter" || key.eq_ignore_ascii_case("y"));
                let no = key == "escape" || (this.yn_keys && key.eq_ignore_ascii_case("n"));
                if yes {
                    (this.yes)(window, cx);
                }
                if yes || no {
                    cx.emit(DialogEvent::Close);
                    cx.stop_propagation();
                }
            }))
            .child(k.text(self.text.clone()))
            .child(
                k.buttons()
                    .child(k.button(
                        "confirm-yes",
                        "Yes",
                        Btn::Plain,
                        Some(Box::new(cx.listener(|this, _, window, cx| {
                            (this.yes)(window, cx);
                            cx.emit(DialogEvent::Close);
                        }))),
                    ))
                    .child(k.button("confirm-no", "No", Btn::Plain, Some(Box::new(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close)))))),
            )
            .into_any_element()
    }
}

// ---- Custom Text Reveal Speed -------------------------------------------------------

/// What OK does with the number.
pub type OnSpeed = Box<dyn Fn(u32, &mut Window, &mut App)>;
/// What Cancel and Esc do.
pub type OnCancel = Box<dyn Fn(&mut Window, &mut App)>;

/// The number the Settings' Text Reveal "Custom" or "New Only" asks for (the console's
/// reveal_speed popup, app.js `showRevealSpeedDialog`): OK or Enter takes it, Cancel or Esc
/// puts the choice before back; a number outside `TextReveal::CUSTOM_RANGE` complains and
/// stays.
pub struct RevealSpeed {
    main: WeakEntity<ClayView>,
    /// The prompt's title, for the entry that asked (`TextReveal::speed_title`).
    title: &'static str,
    input: Entity<InputState>,
    ok: OnSpeed,
    cancel: OnCancel,
    pub error: Option<String>,
}

impl RevealSpeed {
    pub fn new(main: WeakEntity<ClayView>, entry: &str, current: Option<u32>, ok: OnSpeed, cancel: OnCancel, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let value = current.map(|cps| cps.to_string()).unwrap_or_default();
        let input = cx.new(|cx| InputState::new(window, cx).default_value(value).placeholder("1 to 10000".to_string()));
        cx.subscribe_in(&input, window, |this, _, ev: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { .. } = ev {
                this.accept(window, cx);
            }
        })
        .detach();
        Self { main, title: crate::TextReveal::speed_title(entry), input, ok, cancel, error: None }
    }

    fn accept(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let typed = self.input.read(cx).value().to_string();
        match crate::TextReveal::parse_custom(&typed) {
            Some(cps) => {
                (self.ok)(cps, window, cx);
                cx.emit(DialogEvent::Close);
            }
            None => {
                self.error = Some(crate::TextReveal::CUSTOM_PROBLEM.to_string());
                cx.notify();
            }
        }
    }

    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        (self.cancel)(window, cx);
        cx.emit(DialogEvent::Close);
    }
}

impl EventEmitter<DialogEvent> for RevealSpeed {}
impl Focusable for RevealSpeed {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.read(cx).focus_handle(cx)
    }
}

impl Render for RevealSpeed {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(k) = kit(&self.main, cx) else { return div().into_any_element() };
        k.panel("reveal-speed")
            .w(px(360.0))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                if ev.keystroke.key.as_str() == "escape" {
                    this.dismiss(window, cx);
                    cx.stop_propagation();
                }
            }))
            .child(k.heading(self.title))
            .child(k.field("Characters a second:", &self.input))
            .child(k.error(self.error.as_deref()))
            .child(
                k.buttons()
                    .child(k.button("reveal-speed-cancel", "Cancel", Btn::Plain, Some(Box::new(cx.listener(|this, _, w, cx| this.dismiss(w, cx))))))
                    .child(k.button("reveal-speed-ok", "OK", Btn::Primary, Some(Box::new(cx.listener(|this, _, w, cx| this.accept(w, cx)))))),
            )
            .into_any_element()
    }
}

// ---- A dialog's "?" -----------------------------------------------------------------

/// app.js `popupHelpTexts[key]`, read out of the web client itself so the two never
/// drift: an object of `key: [ 'line', ... ]` arrays of single-quoted strings.
pub fn help_text(key: &str) -> Option<Vec<String>> {
    let js = crate::http::WEB_APP_JS;
    let start = js.find("const popupHelpTexts = {")?;
    let body = &js[start..];
    let end = body.find("\n    };")?;
    let body = &body[..end];
    let quoted = format!("'{key}': [");
    let bare = format!("\n        {key}: [");
    let at = body.find(&bare).map(|i| i + bare.len()).or_else(|| body.find(&quoted).map(|i| i + quoted.len()))?;
    let mut lines = Vec::new();
    let mut chars = body[at..].chars();
    while let Some(c) = chars.next() {
        match c {
            ']' => return Some(lines),
            '\'' => {
                let mut s = String::new();
                loop {
                    match chars.next()? {
                        '\\' => s.push(chars.next()?),
                        '\'' => break,
                        c => s.push(c),
                    }
                }
                lines.push(s);
            }
            _ => {}
        }
    }
    None
}

pub struct Help {
    main: WeakEntity<ClayView>,
    focus: FocusHandle,
    lines: Vec<String>,
}

impl Help {
    pub fn new(main: WeakEntity<ClayView>, lines: Vec<String>, cx: &mut Context<Self>) -> Self {
        Self { main, focus: cx.focus_handle(), lines }
    }
}

impl EventEmitter<DialogEvent> for Help {}
impl Focusable for Help {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Help {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(k) = kit(&self.main, cx) else { return div().into_any_element() };
        let max_w = (f32::from(window.viewport_size().width) * 0.85).min(480.0);
        k.panel("popup-help")
            .track_focus(&self.focus)
            .p(px(16.0))
            .max_w(px(max_w))
            .max_h(window.viewport_size().height * 0.9)
            .on_key_down(cx.listener(keys::<Self>(Some(|_, _, cx| cx.emit(DialogEvent::Close)))))
            .child(k.header("popup-help-close", "Help", cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))))
            .child(
                div()
                    .id("popup-help-content")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(8.0))
                    .py(px(4.0))
                    .font_family(k.fonts.primary.clone())
                    .text_size(px(13.0))
                    .line_height(relative(1.5))
                    .children(self.lines.iter().map(|l| div().min_h(px(19.5)).whitespace_normal().child(SharedString::from(l.clone())))),
            )
            .child(k.buttons().child(k.button("popup-help-ok", "Ok", Btn::Primary, Some(Box::new(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close)))))))
            .into_any_element()
    }
}

// ---- A MUD world's changed certificate ----------------------------------------------

/// The server refused a MUD world's TLS certificate (it no longer matches its pin):
/// trust the new one, or don't connect.
pub struct CertChanged {
    main: WeakEntity<ClayView>,
    focus: FocusHandle,
    world_index: usize,
    host: String,
    old: String,
    new: String,
}

impl CertChanged {
    pub fn new(main: WeakEntity<ClayView>, world_index: usize, host: String, old: String, new: String, cx: &mut Context<Self>) -> Self {
        Self { main, focus: cx.focus_handle(), world_index, host, old, new }
    }

    fn trust(&mut self, cx: &mut Context<Self>) {
        let msg = WsMessage::TrustCertificate { world_index: self.world_index, host: self.host.clone(), new_fingerprint: self.new.clone() };
        let _ = self.main.update(cx, |v, cx| v.send_message(msg, cx));
        cx.emit(DialogEvent::Close);
    }
}

impl EventEmitter<DialogEvent> for CertChanged {}
impl Focusable for CertChanged {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for CertChanged {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(k) = kit(&self.main, cx) else { return div().into_any_element() };
        k.panel("cert-mismatch")
            .track_focus(&self.focus)
            .max_w(px(500.0))
            .border_2()
            .border_color(k.t.error)
            .on_key_down(cx.listener(keys::<Self>(None)))
            .child(k.heading_in("TLS Certificate Changed", k.t.error))
            .child(div().child(k.rich(&[
                ("The certificate for ", false),
                (&self.host, true),
                (" no longer matches the one pinned on first connect. This could mean the server was reinstalled, or that someone is intercepting your connection.", false),
            ])))
            .child(k.mono(format!("Old: {}", self.old)))
            .child(k.mono(format!("New: {}", self.new)))
            .child(
                k.buttons()
                    .child(k.button("cert-cancel", "Cancel", Btn::Plain, Some(Box::new(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))))))
                    .child(k.button("cert-trust", "Trust new certificate", Btn::Danger, Some(Box::new(cx.listener(|this, _, _, cx| this.trust(cx)))))),
            )
            .into_any_element()
    }
}

// ---- /import ------------------------------------------------------------------------

/// What an import was sent with, kept for resending it over plaintext if the user agrees.
#[derive(Clone, Debug)]
pub struct ImportCreds {
    pub addr: String,
    pub password: Option<String>,
    pub auth_key: Option<String>,
}

/// Pull worlds, theme and keybindings from another Clay (`ImportSettings`; the result
/// arrives as `ImportResult`, or `ImportNeedsInsecureConfirm` first).
pub struct Import {
    main: WeakEntity<ClayView>,
    addr: Entity<InputState>,
    password: Entity<InputState>,
    key: Entity<InputState>,
}

impl Import {
    pub fn new(main: WeakEntity<ClayView>, prefill: &str, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let addr = cx.new(|cx| InputState::new(window, cx).default_value(prefill.to_string()));
        let password = cx.new(|cx| InputState::new(window, cx).masked(true));
        let key = cx.new(|cx| InputState::new(window, cx).masked(true));
        for f in [&addr, &password, &key] {
            cx.subscribe_in(f, window, |this, _, ev: &InputEvent, _window, cx| {
                if let InputEvent::PressEnter { .. } = ev {
                    this.submit(cx);
                }
            })
            .detach();
        }
        Self { main, addr, password, key }
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        let addr = self.addr.read(cx).value().trim().to_string();
        let password = Some(self.password.read(cx).value().to_string()).filter(|s| !s.is_empty());
        let auth_key = Some(self.key.read(cx).value().to_string()).filter(|s| !s.is_empty());
        if addr.is_empty() {
            return;
        }
        if password.is_none() && auth_key.is_none() {
            let _ = self.main.update(cx, |v, cx| v.client_line("Enter a password or an auth key.", cx));
            return;
        }
        let creds = ImportCreds { addr: addr.clone(), password: password.clone(), auth_key: auth_key.clone() };
        let msg = WsMessage::ImportSettings { addr, password, auth_key, allow_insecure: false };
        let _ = self.main.update(cx, |v, cx| {
            v.surf.pending_import = Some(creds);
            v.send_message(msg, cx);
        });
        cx.emit(DialogEvent::Close);
    }
}

impl EventEmitter<DialogEvent> for Import {}
impl Focusable for Import {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.addr.read(cx).focus_handle(cx)
    }
}

impl Render for Import {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(k) = kit(&self.main, cx) else { return div().into_any_element() };
        k.panel("import")
            .w(px(420.0))
            .on_key_down(cx.listener(keys::<Self>(None)))
            .child(div().text_size(px(16.0)).font_weight(FontWeight::BOLD).child("Import Settings"))
            .child(k.note("Pull worlds, theme, and keybindings from another Clay instance. Remote values win on conflicts; everything else you have locally is kept."))
            .child(k.field("Host[:port]", &self.addr))
            .child(k.field("Password", &self.password))
            .child(k.field("Auth key (optional, instead of password)", &self.key))
            .child(
                k.buttons()
                    .child(k.button("import-cancel", "Cancel", Btn::Plain, Some(Box::new(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))))))
                    .child(k.button("import-go", "Import", Btn::Primary, Some(Box::new(cx.listener(|this, _, _, cx| this.submit(cx)))))),
            )
            .into_any_element()
    }
}

/// The import's target has no TLS: send the password or key unencrypted only if the user
/// says so.
pub struct ImportInsecure {
    main: WeakEntity<ClayView>,
    focus: FocusHandle,
    addr: String,
}

impl ImportInsecure {
    pub fn new(main: WeakEntity<ClayView>, addr: String, cx: &mut Context<Self>) -> Self {
        Self { main, focus: cx.focus_handle(), addr }
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        let _ = self.main.update(cx, |v, _| v.surf.pending_import = None);
        cx.emit(DialogEvent::Close);
    }

    fn go(&mut self, cx: &mut Context<Self>) {
        let _ = self.main.update(cx, |v, cx| {
            if let Some(c) = v.surf.pending_import.take() {
                v.send_message(WsMessage::ImportSettings { addr: c.addr, password: c.password, auth_key: c.auth_key, allow_insecure: true }, cx);
            }
        });
        cx.emit(DialogEvent::Close);
    }
}

impl EventEmitter<DialogEvent> for ImportInsecure {}
impl Focusable for ImportInsecure {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ImportInsecure {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(k) = kit(&self.main, cx) else { return div().into_any_element() };
        k.panel("import-insecure")
            .track_focus(&self.focus)
            .max_w(px(460.0))
            .border_2()
            .border_color(k.t.error)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _w, cx| {
                if ev.keystroke.key == "escape" {
                    this.cancel(cx);
                    cx.stop_propagation();
                }
            }))
            .child(k.heading_in("No Secure Connection", k.t.error))
            .child(div().child(k.rich(&[
                (&format!("{} did not accept a TLS connection. Continuing will send your password/auth-key to it ", self.addr), false),
                ("unencrypted", true),
                (". Only do this on a network you trust.", false),
            ])))
            .child(
                k.buttons()
                    .child(k.button("import-insecure-cancel", "Cancel", Btn::Plain, Some(Box::new(cx.listener(|this, _, _, cx| this.cancel(cx))))))
                    .child(k.button("import-insecure-go", "Send unencrypted", Btn::Danger, Some(Box::new(cx.listener(|this, _, _, cx| this.go(cx)))))),
            )
            .into_any_element()
    }
}

// ---- Recent Worlds ------------------------------------------------------------------

/// The recent_worlds action (the console's Recent Worlds popup): up to six other worlds by
/// their last line to or from the MUD, newest first; Enter or a click switches to one.
pub struct RecentWorlds {
    main: WeakEntity<ClayView>,
    focus: FocusHandle,
    selected: usize,
}

impl RecentWorlds {
    pub fn new(main: WeakEntity<ClayView>, cx: &mut Context<Self>) -> Self {
        Self { main, focus: cx.focus_handle(), selected: 0 }
    }

    /// (world index, name, how long ago), as the console lists them.
    fn entries(&self, cx: &App) -> Vec<(usize, String, String)> {
        let Some(m) = self.main.upgrade() else { return Vec::new() };
        let core = &m.read(cx).session.core;
        core.recent_worlds(std::time::Instant::now())
            .into_iter()
            .map(|(i, ago)| {
                let name = crate::util::strip_ansi_codes(&core.worlds[i].name).trim().to_string();
                (i, name, crate::popup::definitions::connections::format_elapsed(Some(ago.as_secs())))
            })
            .collect()
    }

    fn choose(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(m) = self.main.upgrade() {
            m.update(cx, |m, cx| m.switch_to(index, cx));
        }
        cx.emit(DialogEvent::Close);
    }
}

impl EventEmitter<DialogEvent> for RecentWorlds {}
impl Focusable for RecentWorlds {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for RecentWorlds {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(k) = kit(&self.main, cx) else { return div().into_any_element() };
        let t = k.t.clone();
        let entries = self.entries(cx);
        self.selected = self.selected.min(entries.len().saturating_sub(1));
        let rows: Vec<gpui::AnyElement> = if entries.is_empty() {
            vec![k.note("No recently active worlds.").into_any_element()]
        } else {
            entries
                .iter()
                .enumerate()
                .map(|(n, (i, name, ago))| {
                    let i = *i;
                    let selected = n == self.selected;
                    let hover = t.hover;
                    let row = div()
                        .id(("recent-world", n))
                        .flex()
                        .justify_between()
                        .gap(px(24.0))
                        .px(px(12.0))
                        .py(px(6.0))
                        .rounded(px(4.0))
                        .cursor_pointer()
                        .child(div().min_w_0().overflow_hidden().whitespace_nowrap().child(SharedString::from(name.clone())))
                        .child(div().flex_shrink_0().opacity(0.7).child(SharedString::from(ago.clone())))
                        .on_click(cx.listener(move |this, _, _w, cx| this.choose(i, cx)));
                    if selected { row.bg(t.accent).text_color(gpui::black()) } else { row.hover(move |s| s.bg(hover)) }.into_any_element()
                })
                .collect()
        };
        let indices: Vec<usize> = entries.iter().map(|e| e.0).collect();
        k.panel("recent-worlds")
            .track_focus(&self.focus)
            .min_w(px(300.0))
            .max_w(px(420.0))
            .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, _w, cx| {
                match ev.keystroke.key.as_str() {
                    "escape" => cx.emit(DialogEvent::Close),
                    "up" | "down" => {
                        let by = if ev.keystroke.key == "up" { -1 } else { 1 };
                        if let Some(at) = crate::native_gui::ui::kit::list_step(Some(this.selected), indices.len(), by) {
                            this.selected = at;
                        }
                        cx.notify();
                    }
                    "enter" => match indices.get(this.selected) {
                        Some(&i) => this.choose(i, cx),
                        None => cx.emit(DialogEvent::Close),
                    },
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .child(k.header("recent-worlds-close", "Recent Worlds", cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))))
            .child(div().flex().flex_col().gap(px(2.0)).py(px(4.0)).children(rows))
            .child(k.note("↑↓ select, Enter switch, Esc close"))
            .into_any_element()
    }
}

// ---- New Indicator's Custom background ------------------------------------------------

/// The scheme picked: its index in `newbg::SCHEMES`.
pub type OnScheme = Box<dyn Fn(usize, &mut Window, &mut App)>;

/// "Custom Background" (`NewIndicator::PICKER_TITLE`; the console's glow_scheme popup, app.js
/// `showGlowSchemeDialog`): the schemes of `newbg::SCHEMES`, each drawn as
/// `newbg::SAMPLE_WORD` over its four cells, fading to this window's background. A click or
/// Enter picks one; Esc or Cancel leaves the choice as it was.
pub struct GlowScheme {
    main: WeakEntity<ClayView>,
    focus: FocusHandle,
    selected: usize,
    ok: OnScheme,
    cancel: OnCancel,
}

impl GlowScheme {
    pub fn new(main: WeakEntity<ClayView>, current: usize, ok: OnScheme, cancel: OnCancel, cx: &mut Context<Self>) -> Self {
        Self { main, focus: cx.focus_handle(), selected: current.min(crate::newbg::SCHEMES.len() - 1), ok, cancel }
    }

    fn choose(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        (self.ok)(i, window, cx);
        cx.emit(DialogEvent::Close);
    }

    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        (self.cancel)(window, cx);
        cx.emit(DialogEvent::Close);
    }
}

impl EventEmitter<DialogEvent> for GlowScheme {}
impl Focusable for GlowScheme {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for GlowScheme {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(k) = kit(&self.main, cx) else { return div().into_any_element() };
        let t = k.t.clone();
        // What the glow fades to: this window's output background.
        let to = self.main.upgrade().map_or(crate::reveal::Rgb(0, 0, 0), |m| crate::reveal::Rgb::from_array(m.read(cx).palette.bg));
        let mono = k.fonts.primary.clone();
        let rows: Vec<gpui::AnyElement> = crate::newbg::SCHEMES
            .iter()
            .enumerate()
            .map(|(n, s)| {
                let selected = n == self.selected;
                let hover = t.hover;
                let cells = crate::newbg::swatch(s.start, to);
                let word = div().flex().font_family(mono.clone()).font_weight(FontWeight::BOLD).children(
                    crate::newbg::SAMPLE_WORD.chars().zip(cells).map(|(ch, bg)| {
                        div()
                            .w(px(14.0))
                            .flex()
                            .justify_center()
                            .bg(crate::native_gui::ui::output::hsla(bg.array()))
                            .text_color(crate::native_gui::ui::output::hsla(crate::newbg::text_on(bg).array()))
                            .child(SharedString::from(ch.to_string()))
                    }),
                );
                let row = div()
                    .id(("glow-scheme", n))
                    .flex()
                    .items_center()
                    .gap(px(16.0))
                    .px(px(12.0))
                    .py(px(5.0))
                    .rounded(px(4.0))
                    .cursor_pointer()
                    .child(word)
                    .child(div().child(SharedString::from(s.label)))
                    .on_click(cx.listener(move |this, _, w, cx| this.choose(n, w, cx)));
                if selected { row.bg(t.accent).text_color(gpui::black()) } else { row.hover(move |s| s.bg(hover)) }.into_any_element()
            })
            .collect();
        let count = crate::newbg::SCHEMES.len();
        k.panel("glow-scheme")
            .track_focus(&self.focus)
            .w(px(300.0))
            .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, w, cx| {
                match ev.keystroke.key.as_str() {
                    "escape" => this.dismiss(w, cx),
                    "up" | "down" => {
                        let by = if ev.keystroke.key == "up" { -1 } else { 1 };
                        if let Some(at) = crate::native_gui::ui::kit::list_step(Some(this.selected), count, by) {
                            this.selected = at;
                        }
                        cx.notify();
                    }
                    "enter" => {
                        let i = this.selected;
                        this.choose(i, w, cx);
                    }
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .child(k.heading(crate::NewIndicator::PICKER_TITLE))
            .child(div().flex().flex_col().gap(px(2.0)).py(px(4.0)).children(rows))
            .child(k.note("↑↓ select, Enter choose, Esc cancel"))
            .child(
                k.buttons()
                    .child(k.button("glow-scheme-cancel", "Cancel", Btn::Plain, Some(Box::new(cx.listener(|this, _, w, cx| this.dismiss(w, cx))))))
                    .child(k.button("glow-scheme-ok", "OK", Btn::Primary, Some(Box::new(cx.listener(|this, _, w, cx| {
                        let i = this.selected;
                        this.choose(i, w, cx);
                    }))))),
            )
            .into_any_element()
    }
}

// ---- Notes List ---------------------------------------------------------------------

/// `/note -l` (the console's notes list, app.js `openNotesList`): the worlds with notes and
/// each one's first line; Enter or a click opens that world's notes.
pub struct NotesList {
    main: WeakEntity<ClayView>,
    focus: FocusHandle,
    /// (world index, name, first line)
    entries: Vec<(usize, String, String)>,
    selected: usize,
}

impl NotesList {
    pub fn new(main: WeakEntity<ClayView>, entries: Vec<(usize, String, String)>, current: usize, cx: &mut Context<Self>) -> Self {
        let selected = entries.iter().position(|e| e.0 == current).unwrap_or(0);
        Self { main, focus: cx.focus_handle(), entries, selected }
    }

    fn choose(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(m) = self.main.upgrade() {
            m.update(cx, |m, cx| m.open_notes_for(index, cx));
        }
        cx.emit(DialogEvent::Close);
    }
}

impl EventEmitter<DialogEvent> for NotesList {}
impl Focusable for NotesList {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for NotesList {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(k) = kit(&self.main, cx) else { return div().into_any_element() };
        let t = k.t.clone();
        self.selected = self.selected.min(self.entries.len().saturating_sub(1));
        let rows: Vec<gpui::AnyElement> = self.entries
            .iter()
            .enumerate()
            .map(|(n, (i, name, preview))| {
                let i = *i;
                let hover = t.hover;
                let row = div()
                    .id(("notes-list", n))
                    .flex()
                    .justify_between()
                    .gap(px(24.0))
                    .px(px(12.0))
                    .py(px(6.0))
                    .rounded(px(4.0))
                    .cursor_pointer()
                    .child(div().flex_shrink_0().child(SharedString::from(name.clone())))
                    .child(div().min_w_0().overflow_hidden().whitespace_nowrap().opacity(0.7).child(SharedString::from(preview.clone())))
                    .on_click(cx.listener(move |this, _, _w, cx| this.choose(i, cx)));
                if n == self.selected { row.bg(t.accent).text_color(gpui::black()) } else { row.hover(move |s| s.bg(hover)) }.into_any_element()
            })
            .collect();
        let indices: Vec<usize> = self.entries.iter().map(|e| e.0).collect();
        k.panel("notes-list")
            .track_focus(&self.focus)
            .min_w(px(340.0))
            .max_w(px(560.0))
            .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, _w, cx| {
                match ev.keystroke.key.as_str() {
                    "escape" => cx.emit(DialogEvent::Close),
                    "up" | "down" => {
                        let by = if ev.keystroke.key == "up" { -1 } else { 1 };
                        if let Some(at) = crate::native_gui::ui::kit::list_step(Some(this.selected), indices.len(), by) {
                            this.selected = at;
                        }
                        cx.notify();
                    }
                    "enter" => match indices.get(this.selected) {
                        Some(&i) => this.choose(i, cx),
                        None => cx.emit(DialogEvent::Close),
                    },
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .child(k.header("notes-list-close", "World Notes", cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))))
            .child(div().flex().flex_col().gap(px(2.0)).py(px(4.0)).children(rows))
            .child(k.note("↑↓ select, Enter open, Esc close"))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_help_texts_come_out_of_app_js() {
        let world = help_text("worldEditor").expect("world editor help");
        assert!(world.iter().any(|l| !l.is_empty()));
        assert!(help_text("clay-server").is_some(), "a quoted key");
        assert!(help_text("nonesuch").is_none());
        // Every key a dialog asks for exists (Settings' Font tab asked for "font", which
        // had none, so its "?" did nothing: B27).
        for key in ["setup", "web", "font", "clay-server", "worldEditor", "worldSelector", "actionsList", "actionEditor", "connections"] {
            assert!(help_text(key).is_some_and(|l| !l.is_empty()), "{key}");
        }
        // The Web tab's help describes the tab as it is now.
        let web = help_text("web").expect("web help").join("\n");
        for field in ["Port:", "Web Path:", "Remote Lines:", "Allow List:", "Password:", "Custom Cert File:", "Auth Key:", "Remote Access:"] {
            assert!(web.contains(field), "web help has no {field}");
        }
        for gone in ["Protocol:", "HTTP Enabled", "HTTP Port"] {
            assert!(!web.contains(gone), "web help still describes {gone}");
        }
        assert!(help_text("font").expect("font help").iter().any(|l| l.starts_with("Input Ht:")));
    }

    #[test]
    fn password_checks_are_the_web_clients() {
        assert_eq!(password_problem("", "abcd", "abcd"), Some("All fields are required"));
        assert_eq!(password_problem("old", "abcd", "abce"), Some("New passwords do not match"));
        assert_eq!(password_problem("old", "abc", "abc"), Some("New password must be at least 4 characters"));
        assert_eq!(password_problem("old", "abcd", "abcd"), None);
    }
}
