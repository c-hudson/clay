//! Dialogs over a window's content: the web client's modals (index.html `.modal`) and the
//! dialogs app.js builds, one stack per window. A dialog is a view of its own; it closes
//! itself by emitting `Close`, and talks to the server through the main view.

pub mod emoji;
pub mod small;
pub mod web;

use gpui::{
    AnyElement, AnyView, AppContext, Context, Entity, EntityId, EventEmitter, FocusHandle, Focusable, IntoElement, ParentElement, Render, Styled, Subscription,
    Window, div, prelude::*,
};

use super::chrome::alpha;

/// What a dialog tells the window it's in.
pub enum DialogEvent {
    Close,
}

/// Which dialog: one of each is open at most (opening one again replaces it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    ConnectionLog,
    Reconnect,
    Password,
    Menu,
    /// The "?" of a dialog (app.js `popupHelpTexts`): over the dialog it explains.
    Help,
    CertMismatch,
    Import,
    ImportInsecure,
    Emoji,
    Confirm,
    ModifyKey,
    RemoteAccess,
}

struct Open {
    kind: Kind,
    view: AnyView,
    id: EntityId,
    /// Where the keyboard goes when this one is on top again.
    focus: FocusHandle,
    _closed: Subscription,
}

#[derive(Default)]
pub struct Stack {
    open: Vec<Open>,
}

/// A window that shows dialogs.
pub trait Host: Sized + 'static {
    fn dialogs(&mut self) -> &mut Stack;
    /// Where the keyboard goes when the last dialog closes.
    fn restore_focus(&mut self, window: &mut Window, cx: &mut Context<Self>);
}

impl Stack {
    pub fn is_open(&self, kind: &Kind) -> bool {
        self.open.iter().any(|o| &o.kind == kind)
    }

    pub fn any(&self) -> bool {
        !self.open.is_empty()
    }

    /// The open dialog of this kind, as its own view type.
    pub fn get<V: 'static>(&self, kind: &Kind) -> Option<Entity<V>> {
        self.open.iter().find(|o| &o.kind == kind).and_then(|o| o.view.clone().downcast::<V>().ok())
    }

    pub fn close(&mut self, kind: &Kind) {
        self.open.retain(|o| &o.kind != kind);
    }

    fn remove(&mut self, id: EntityId) {
        self.open.retain(|o| o.id != id);
    }

    /// The dialogs, the newest on top, over a dimmed window. The web's modals leave the
    /// window behind them clickable; these don't, so a click meant for the dialog can't
    /// land in the output.
    pub fn render(&self) -> Option<AnyElement> {
        if self.open.is_empty() {
            return None;
        }
        Some(
            div()
                .absolute()
                .inset_0()
                .children(self.open.iter().enumerate().map(|(i, o)| {
                    div()
                        .id(("dialog-layer", i))
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(alpha([0, 0, 0], if i == 0 { 0.35 } else { 0.2 }))
                        .occlude()
                        .child(o.view.clone())
                }))
                .into_any_element(),
        )
    }
}

/// Open `view` as a dialog of `kind` (replacing one of that kind), with the keyboard in it.
pub fn open<H: Host, V: Render + EventEmitter<DialogEvent> + Focusable>(host: &mut H, kind: Kind, view: Entity<V>, window: &mut Window, cx: &mut Context<H>) {
    push(host, kind, view, true, window, cx);
}

/// The same, leaving the keyboard where it is (a dialog that comes and goes on its own).
pub fn open_quietly<H: Host, V: Render + EventEmitter<DialogEvent> + Focusable>(host: &mut H, kind: Kind, view: Entity<V>, window: &mut Window, cx: &mut Context<H>) {
    push(host, kind, view, false, window, cx);
}

fn push<H: Host, V: Render + EventEmitter<DialogEvent> + Focusable>(host: &mut H, kind: Kind, view: Entity<V>, take_focus: bool, window: &mut Window, cx: &mut Context<H>) {
    host.dialogs().close(&kind);
    let closed = cx.subscribe_in(&view, window, |host: &mut H, view, _: &DialogEvent, window, cx| {
        host.dialogs().remove(view.entity_id());
        match host.dialogs().open.last().map(|o| o.focus.clone()) {
            Some(under) => window.focus(&under, cx),
            None => host.restore_focus(window, cx),
        }
        cx.notify();
    });
    let focus = view.focus_handle(cx);
    if take_focus {
        window.focus(&focus, cx);
    }
    let id = view.entity_id();
    host.dialogs().open.push(Open { kind, view: view.into(), id, focus, _closed: closed });
    cx.notify();
}

/// A dialog's "?" in whichever window `host` is.
pub fn open_help<H: Host>(host: &mut H, main: gpui::WeakEntity<super::ClayView>, key: &str, window: &mut Window, cx: &mut Context<H>) {
    let Some(lines) = small::help_text(key) else { return };
    let v = cx.new(|cx| small::Help::new(main, lines, cx));
    open(host, Kind::Help, v, window, cx);
}

/// Ask a yes/no question; `yes` runs on Yes. `yn_keys`: Y and Enter answer Yes, N No.
pub fn confirm<H: Host>(
    host: &mut H,
    main: gpui::WeakEntity<super::ClayView>,
    text: String,
    yn_keys: bool,
    yes: impl Fn(&mut Window, &mut gpui::App) + 'static,
    window: &mut Window,
    cx: &mut Context<H>,
) {
    let v = cx.new(|cx| {
        let mut c = small::Confirm::new(main, text, Box::new(yes), cx);
        c.yn_keys = yn_keys;
        c
    });
    open(host, Kind::Confirm, v, window, cx);
}
