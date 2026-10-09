//! The GUI's other windows: Settings, Help, Actions and Worlds open in OS windows of
//! their own (webview_gui.rs's popup windows). Unlike the WebView's, these don't log in
//! again: they share the main window's connection.
//! - One window per kind; asking again brings it forward with what was asked for.
//! - A window closes when its content does.

pub mod actions;
pub mod help;
pub mod keybind_editor;
pub mod notes;
pub mod settings;
pub mod theme_editor;
pub mod worlds;

use std::collections::HashMap;

use gpui::{
    AnyView, App, AppContext, Bounds, Context, EventEmitter, FocusHandle, Focusable, IntoElement, ParentElement, Render, Styled, Subscription,
    TitlebarOptions, WeakEntity, Window, WindowBounds, WindowHandle, WindowOptions, div, px, size,
};

use super::ClayView;
use super::dialogs::{self, DialogEvent, Host};

/// The kinds (app.js `openInPopupWindow`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Settings,
    Help,
    Actions,
    Worlds,
    /// A world's notes (one window per world).
    Notes(usize),
    ThemeEditor,
    KeybindEditor,
}

impl Kind {
    /// The window's title and size (webview_gui.rs `popup_window_spec`).
    fn spec(self) -> (&'static str, f32, f32) {
        match self {
            Kind::Settings => ("Clay - Settings", 520.0, 680.0),
            Kind::Help => ("Clay - Help", 640.0, 720.0),
            Kind::Actions => ("Clay - Actions", 760.0, 620.0),
            Kind::Worlds => ("Clay - Worlds", 640.0, 680.0),
            // The world's name goes in the title once the notes arrive.
            Kind::Notes(_) => ("Clay - Notes", 700.0, 500.0),
            Kind::ThemeEditor => ("Clay - Theme Editor", 1000.0, 720.0),
            Kind::KeybindEditor => ("Clay - Keybind Editor", 1000.0, 700.0),
        }
    }
}

/// What the main view tells its windows.
pub enum MainEvent {
    /// The theme, fonts or settings changed: redraw.
    Restyled,
    /// The actions changed.
    Actions,
    /// What the Worlds window shows changed.
    Worlds,
    /// The settings changed.
    Settings,
}

impl EventEmitter<MainEvent> for ClayView {}

/// A window's frame: its content full size, and the dialogs over it.
pub struct PopupWindow {
    main: WeakEntity<ClayView>,
    content: AnyView,
    content_focus: FocusHandle,
    stack: dialogs::Stack,
    _subs: Vec<Subscription>,
}

impl PopupWindow {
    fn new<V: Render + EventEmitter<DialogEvent> + Focusable>(main: WeakEntity<ClayView>, content: gpui::Entity<V>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut subs = Vec::new();
        // Its content closing closes the window.
        subs.push(cx.subscribe_in(&content, window, |_, _, _: &DialogEvent, window, _cx| window.remove_window()));
        if let Some(m) = main.upgrade() {
            subs.push(cx.subscribe(&m, |_, _, _: &MainEvent, cx| cx.notify()));
        }
        let content_focus = content.focus_handle(cx);
        window.focus(&content_focus, cx);
        Self { main, content: content.into(), content_focus, stack: Default::default(), _subs: subs }
    }

    /// The content, as its own type.
    pub fn content<V: 'static>(&self) -> Option<gpui::Entity<V>> {
        self.content.clone().downcast::<V>().ok()
    }
}

impl Host for PopupWindow {
    fn dialogs(&mut self) -> &mut dialogs::Stack {
        &mut self.stack
    }

    fn restore_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.content_focus, cx);
    }
}

impl Render for PopupWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bg = self.main.upgrade().map(|m| m.read(cx).kit().t.elevated).unwrap_or(gpui::black());
        div().size_full().relative().bg(bg).child(self.content.clone()).children(self.stack.render())
    }
}

/// The windows the main view has open.
#[derive(Default)]
pub struct Windows {
    open: HashMap<Kind, WindowHandle<gpui_component::Root>>,
    /// Each one's frame, to reach its content.
    frames: HashMap<Kind, WeakEntity<PopupWindow>>,
}

impl Windows {
    /// The frame of an open window of `kind`, if it is still open.
    pub fn frame(&self, kind: Kind) -> Option<gpui::Entity<PopupWindow>> {
        self.frames.get(&kind).and_then(|f| f.upgrade())
    }

    #[cfg(feature = "native-gui-snapshot")]
    pub fn handle(&self, kind: Kind) -> Option<gpui::AnyWindowHandle> {
        self.handle_of(kind)
    }

    /// The window of `kind`, to act in it.
    pub fn handle_of(&self, kind: Kind) -> Option<gpui::AnyWindowHandle> {
        self.open.get(&kind).map(|h| (*h).into())
    }
}

impl ClayView {
    /// Bring the window of `kind` forward, or open it with `build`'s content. `again`
    /// hands an open one what was asked for this time (webview_gui.rs's `__clayPopupArg`).
    /// Done once this update is over: a new window draws at once, and reads this view.
    pub(super) fn open_window<V: Render + EventEmitter<DialogEvent> + Focusable>(
        &mut self,
        kind: Kind,
        build: impl FnOnce(WeakEntity<ClayView>, &mut Window, &mut App) -> gpui::Entity<V> + 'static,
        again: impl FnOnce(gpui::Entity<V>, &mut Window, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let main = cx.entity().downgrade();
        let existing = self.windows.open.get(&kind).copied().zip(self.windows.frame(kind));
        cx.defer(move |cx| {
            if let Some((handle, frame)) = existing {
                let shown = handle.update(cx, |_, window, cx| {
                    window.activate_window();
                    if let Some(content) = frame.read(cx).content::<V>() {
                        again(content, window, cx);
                    }
                });
                if shown.is_ok() {
                    return;
                }
            }
            let (title, w, h) = kind.spec();
            let bounds = Bounds::centered(None, size(px(w), px(h)), cx);
            let mut frame = None;
            let opened = cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions { title: Some(title.into()), ..Default::default() }),
                    ..Default::default()
                },
                |window, cx| {
                    let content = build(main.clone(), window, cx);
                    let f = cx.new(|cx| PopupWindow::new(main.clone(), content, window, cx));
                    frame = Some(f.downgrade());
                    cx.new(|cx| gpui_component::Root::new(f, window, cx))
                },
            );
            match (opened, frame) {
                (Ok(handle), Some(frame)) => {
                    let _ = main.update(cx, |v, _| {
                        v.windows.open.insert(kind, handle);
                        v.windows.frames.insert(kind, frame);
                    });
                }
                _ => crate::debug_log(true, &format!("native-gui: can't open the {title} window")),
            }
        });
    }

    /// The theme or settings changed: the other windows redraw.
    pub(super) fn restyled(&mut self, cx: &mut Context<Self>) {
        cx.emit(MainEvent::Restyled);
    }
}
