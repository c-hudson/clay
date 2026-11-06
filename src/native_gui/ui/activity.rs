//! A paused session (`/pause`, `/remote --pause`) resumes when this GUI is used: a key, a
//! mouse press or a wheel turn in any of its windows, or a choice from the desktop's menu
//! (`ClayView::menu_action`). The core asks the server once (`ResumeSession`), and the
//! server's `PausedState` takes the badge down.

use gpui::{Context, DispatchPhase, MouseDownEvent, ScrollWheelEvent, Styled, Subscription, WeakEntity, Window};

use super::ClayView;

impl ClayView {
    /// The user did something here: a paused session resumes.
    pub(super) fn user_active(&mut self, cx: &mut Context<Self>) {
        if self.session.core.paused {
            let fx = self.session.core.user_active();
            self.after(fx, cx);
        }
    }
}

/// Every keystroke in `window` goes to `view`'s `user_active`, a key a binding takes
/// first (Tab, Copy, a text field's) too, until the subscription is dropped with its
/// window's view.
pub(super) fn watch_keys<V: 'static>(view: WeakEntity<ClayView>, window: &Window, cx: &mut Context<V>) -> Subscription {
    let own = window.window_handle();
    cx.intercept_keystrokes(move |_, window, cx| {
        if window.window_handle() == own {
            let _ = view.update(cx, |v, cx| v.user_active(cx));
        }
    })
}

/// Every mouse press and wheel turn in the window it is drawn in goes to `view`'s
/// `user_active`, before anything in the window handles (or stops) it.
pub(super) fn watch_mouse(view: WeakEntity<ClayView>) -> impl gpui::IntoElement {
    gpui::canvas(
        |_, _, _| (),
        move |_, _, window, _cx| {
            let v = view.clone();
            window.on_mouse_event(move |_: &MouseDownEvent, phase, _window, cx| {
                if phase == DispatchPhase::Capture {
                    let _ = v.update(cx, |v, cx| v.user_active(cx));
                }
            });
            let v = view.clone();
            window.on_mouse_event(move |_: &ScrollWheelEvent, phase, _window, cx| {
                if phase == DispatchPhase::Capture {
                    let _ = v.update(cx, |v, cx| v.user_active(cx));
                }
            });
        },
    )
    .absolute()
    .size_0()
}
