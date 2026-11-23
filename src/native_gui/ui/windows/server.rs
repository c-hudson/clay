//! The Server page (user request 2026-11-21: the Android app's server settings in every
//! program that starts Clay): this machine's default server and its login, which a bare
//! `clay` attaches to (src/clientconf.rs). Settings' Server tab when connected, and a
//! window of its own (`ServerView`, a bare `/server`) when not - so a default server that
//! can't be reached can still be put right from the GUI that failed to reach it.
//!
//! Saved by this GUI to this machine's client.dat, never sent to the Clay it's attached
//! to: the page belongs to the machine that starts Clay, not to the main instance.

use gpui::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, Render, Styled, WeakEntity, Window, div, prelude::*, px, relative,
};
use gpui_component::input::InputState;

use crate::clientconf::{ClientConf, ServerPage};
use crate::native_gui::ui::ClayView;
use crate::native_gui::ui::dialogs::{self, DialogEvent, Host};
use crate::native_gui::ui::kit::{Btn, Kit};

/// The page's five fields.
pub struct ServerForm {
    host: Entity<InputState>,
    port: Entity<InputState>,
    username: Entity<InputState>,
    password: Entity<InputState>,
    auth_key: Entity<InputState>,
}

impl ServerForm {
    /// Filled from client.dat.
    pub fn new(window: &mut Window, cx: &mut App) -> ServerForm {
        let page = ClientConf::load().server_page();
        let field = |v: String, ph: &'static str, masked: bool, window: &mut Window, cx: &mut App| {
            cx.new(|cx| InputState::new(window, cx).masked(masked).default_value(v).placeholder(ph))
        };
        ServerForm {
            host: field(page.host, "Blank: no default server", false, window, cx),
            port: field(page.port, "9000", false, window, cx),
            username: field(page.username, "A multiuser server's only", false, window, cx),
            password: field(page.password, "", true, window, cx),
            auth_key: field(page.auth_key, "Tried before the password", true, window, cx),
        }
    }

    /// What the fields say now.
    pub fn page(&self, cx: &App) -> ServerPage {
        let text = |f: &Entity<InputState>| f.read(cx).value().to_string();
        ServerPage {
            host: text(&self.host).trim().to_string(),
            port: text(&self.port).trim().to_string(),
            username: text(&self.username).trim().to_string(),
            password: text(&self.password),
            auth_key: text(&self.auth_key).trim().to_string(),
        }
    }

    /// Write the page to client.dat. `Ok(Some(..))` is what to say when the default
    /// server changed; `Err` why it can't be saved (a bad port, an unwritable file).
    pub fn save(&self, cx: &App) -> Result<Option<String>, String> {
        let mut conf = ClientConf::load();
        let before = conf.default_server.clone();
        conf.set_server_page(&self.page(cx))?;
        conf.save()?;
        Ok((conf.default_server != before).then(|| crate::clientconf::saved_message(&conf)))
    }

    /// The page's card.
    pub fn view(&self, k: &Kit, error: Option<&str>) -> gpui::AnyElement {
        let input = |e: &Entity<InputState>| k.input(e).font_family(k.fonts.ui.clone()).text_size(px(12.0)).into_any_element();
        let mut card = k
            .card()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(div().text_size(px(12.0)).text_color(k.t.fg2).child(
                "A bare `clay` on this machine attaches to this Clay, in the Preferred Interface. `clay --gui` and `clay --console` run this machine's own.",
            ))
            .child(row(k, "Host", input(&self.host)))
            .child(row(k, "Port", div().w(px(80.0)).child(input(&self.port)).into_any_element()))
            .child(row(k, "Username", input(&self.username)))
            .child(row(k, "Password", input(&self.password)))
            .child(row(k, "Auth Key", input(&self.auth_key)));
        if let Some(e) = error {
            card = card.child(div().pl(px(98.0)).text_size(px(12.0)).text_color(k.t.error).child(e.to_string()));
        }
        card = card.child(div().pt(px(4.0)).text_size(px(11.0)).text_color(k.t.muted).child(
            "Kept on this machine, the password and key encrypted; never sent to the Clay you are attached to.",
        ));
        card.into_any_element()
    }
}

/// `.setting-row`: a right-aligned label and its control (settings.rs's).
fn row(k: &Kit, label: &'static str, value: gpui::AnyElement) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .py(px(2.0))
        .child(div().w(px(90.0)).flex_shrink_0().pr(px(8.0)).text_right().text_size(px(12.0)).text_color(k.t.fg2).child(label))
        .child(div().flex_1().min_w_0().child(value))
}

/// The Server page as a window of its own: a bare `/server` with no connection, when the
/// Settings window (which edits the server's settings) can't open.
pub struct ServerView {
    main: WeakEntity<ClayView>,
    focus: FocusHandle,
    form: ServerForm,
    error: Option<String>,
    stack: dialogs::Stack,
}

impl ServerView {
    pub fn new(main: WeakEntity<ClayView>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let form = ServerForm::new(window, cx);
        Self { main, focus: cx.focus_handle(), form, error: None, stack: Default::default() }
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        match self.form.save(cx) {
            Ok(said) => {
                if let Some(said) = said {
                    let _ = self.main.update(cx, |v, cx| v.client_line(&said, cx));
                }
                cx.emit(DialogEvent::Close);
            }
            Err(e) => {
                self.error = Some(e);
                cx.notify();
            }
        }
    }

    fn help(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let main = self.main.clone();
        dialogs::open_help(self, main, "server", window, cx);
    }
}

impl Host for ServerView {
    fn dialogs(&mut self) -> &mut dialogs::Stack {
        &mut self.stack
    }

    fn restore_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
    }
}

impl EventEmitter<DialogEvent> for ServerView {}
impl Focusable for ServerView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ServerView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(k) = self.main.upgrade().map(|m| m.read(cx).kit()) else { return div().into_any_element() };
        let t = k.t.clone();
        div()
            .id("server-window")
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .font_family(k.fonts.ui.clone())
            .text_size(px(13.0))
            .line_height(relative(1.4))
            .text_color(t.fg)
            .on_key_down(cx.listener(|_, ev: &KeyDownEvent, _w, cx| {
                if ev.keystroke.key == "escape" {
                    cx.emit(DialogEvent::Close);
                    cx.stop_propagation();
                }
            }))
            .child(div().px(px(14.0)).pt(px(10.0)).pb(px(4.0)).child(k.header_with(
                "server-close",
                div().text_size(px(11.0)).font_weight(FontWeight::SEMIBOLD).text_color(t.muted).child("SERVER"),
                cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close)),
            )))
            .child(div().id("server-body").flex_1().min_h_0().overflow_y_scroll().px(px(14.0)).pt(px(8.0)).pb(px(14.0)).child(self.form.view(&k, self.error.as_deref())))
            .child(
                k.buttons()
                    .px(px(14.0))
                    .pt(px(8.0))
                    .pb(px(10.0))
                    .child(k.help_button("server-help", cx.listener(|this, _, w, cx| this.help(w, cx))))
                    .child(div().flex_1())
                    .child(k.button("server-cancel", "Cancel", Btn::Plain, Some(Box::new(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))))))
                    .child(k.button("server-save", "Save", Btn::Primary, Some(Box::new(cx.listener(|this, _, _, cx| this.save(cx)))))),
            )
            .children(self.stack.render())
            .into_any_element()
    }
}
