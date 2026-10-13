//! The GUI's other windows: Settings, Help, Actions, Worlds and the emoji picker open in
//! OS windows of their own (webview_gui.rs's popup windows). Unlike the WebView's, these don't log in
//! again: they share the main window's connection.
//! - One window per kind; asking again brings it forward with what was asked for.
//! - A window closes when its content does.

pub mod action_editor;
pub mod actions;
pub mod emoji;
pub mod help;
pub mod keybind_editor;
pub mod notes;
pub mod settings;
pub mod theme_editor;
pub mod worlds;

use std::collections::HashMap;

use gpui::{
    AnyView, App, AppContext, Bounds, Context, EventEmitter, FocusHandle, Focusable, IntoElement, ParentElement, Pixels, Render, Size, Styled,
    Subscription, TitlebarOptions, WeakEntity, Window, WindowBounds, WindowHandle, WindowKind, WindowOptions, div, px, size,
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
    ActionEditor,
    /// The emoji picker (Esc-e): a window, so it can stay up while several are picked.
    Emoji,
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
            Kind::ActionEditor => ("Clay - Action Editor", 960.0, 700.0),
            Kind::Emoji => ("Clay - Emoji", emoji::WIDTH, emoji::height()),
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
    opacity: super::opacity::Opacity,
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
        Self { main, content: content.into(), content_focus, stack: Default::default(), opacity: Default::default(), _subs: subs }
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bg = self.main.upgrade().map(|m| m.read(cx).kit().t.elevated).unwrap_or(gpui::black());
        // Every window takes the Transparency, as the WebView GUI's do.
        let alpha = self.main.upgrade().map_or(1.0, |m| m.read(cx).transparency());
        self.opacity.apply(window, alpha);
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

/// Where a window `size` big opens: centred on `over` (the window that asked), moved as
/// little as needed to stay inside `screen` (that window's display); `fallback` (the
/// display's centre) when the asking window is unknown. gpui's own default centres on
/// the whole X screen, which with more than one monitor is a seam, and the user ruled
/// the windows open relative to the GUI window.
fn bounds_over(over: Option<Bounds<Pixels>>, screen: Option<Bounds<Pixels>>, size: Size<Pixels>, fallback: Bounds<Pixels>) -> Bounds<Pixels> {
    let Some(over) = over else { return fallback };
    let mut b = Bounds::centered_at(over.center(), size);
    if let Some(s) = screen {
        let max_x = s.origin.x + s.size.width - size.width;
        let max_y = s.origin.y + s.size.height - size.height;
        b.origin.x = b.origin.x.min(max_x).max(s.origin.x);
        b.origin.y = b.origin.y.min(max_y).max(s.origin.y);
    }
    b
}

/// On Linux, a window of ours is a floating one: gpui marks it a transient of the window
/// with the keyboard (WM_TRANSIENT_FOR under X11, the toplevel's parent under Wayland),
/// so the window manager places it over that window and keeps it with it - most ignore
/// a plain window's requested position. Elsewhere a normal window at `bounds_over`'s
/// position, which macOS and Windows honour; macOS's floating kind is a panel above
/// every application's windows, not wanted.
fn popup_kind() -> WindowKind {
    if cfg!(target_os = "linux") { WindowKind::Floating } else { WindowKind::Normal }
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
        // A follower's windows are the main window's (one Settings, one Help...).
        let main = self.owner(cx);
        // The new window opens over the window that asked.
        let asked_from = self.link.own_window;
        cx.defer(move |cx| {
            let existing = main.upgrade().and_then(|m| {
                let m = m.read(cx);
                m.windows.open.get(&kind).copied().zip(m.windows.frame(kind))
            });
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
            let size = size(px(w), px(h));
            let over = asked_from.and_then(|h| cx.update_window(h, |_, window, cx| (window.bounds(), window.display(cx).map(|d| d.visible_bounds()))).ok());
            let bounds = bounds_over(over.map(|o| o.0), over.and_then(|o| o.1), size, Bounds::centered(None, size, cx));
            let mut frame = None;
            let opened = cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions { title: Some(title.into()), ..Default::default() }),
                    kind: popup_kind(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::point;

    fn b(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), size(px(w), px(h)))
    }

    #[test]
    fn a_window_opens_centred_on_the_one_that_asked_and_on_its_screen() {
        let screen = Some(b(0.0, 0.0, 1920.0, 1080.0));
        let fallback = b(1.0, 1.0, 1.0, 1.0);
        let main = Some(b(400.0, 200.0, 800.0, 600.0));
        assert_eq!(bounds_over(main, screen, size(px(420.0), px(373.0)), fallback), b(590.0, 313.5, 420.0, 373.0));
        // Asked from a window at the screen's bottom right: moved just enough to stay on it.
        let corner = Some(b(1600.0, 900.0, 800.0, 600.0));
        assert_eq!(bounds_over(corner, screen, size(px(420.0), px(373.0)), fallback), b(1500.0, 707.0, 420.0, 373.0));
        // Taller than the screen: pinned at its top, never off its edge.
        assert_eq!(bounds_over(main, screen, size(px(420.0), px(1200.0)), fallback).origin, point(px(590.0), px(0.0)));
        // On a second monitor to the right: that monitor's bounds, not the first's (the
        // window's centre is 3900,400; the right edge pushes it back to 3420).
        let right = Some(b(1920.0, 0.0, 1920.0, 1080.0));
        assert_eq!(bounds_over(Some(b(3500.0, 100.0, 800.0, 600.0)), right, size(px(420.0), px(373.0)), fallback).origin, point(px(3420.0), px(213.5)));
        // No window to centre on: the display's centre.
        assert_eq!(bounds_over(None, screen, size(px(10.0), px(10.0)), fallback), fallback);
        // No screen known: centred, unclamped.
        assert_eq!(bounds_over(corner, None, size(px(420.0), px(373.0)), fallback).origin, point(px(1790.0), px(1013.5)));
    }
}

