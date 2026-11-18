//! The size popup: while this window is resized, a small box in its middle says how many
//! characters the output holds (`80x24`), as xfwm4 does for xfce4-terminal (user request
//! 2026-11-15; the rules and their tests are `crate::sizepopup`, app.js
//! `noteWindowResize` is the web page's).
//!
//! Every `ClayView` window has its own - the main one, a `/window`, a `/window --grep` -
//! and the popup windows, which have no output, have none. The label is this window's
//! output as its last layout measured it (`Scroll::view_cols`/`view_rows`, the pair
//! `report_view` sends the server), so it is a frame behind a resize and corrected by the
//! frame that layout asks for; a pane showing the splash is measured by
//! `output::view_probe`. **Split (`/split`), it is the output as if it weren't** (user
//! request 2026-11-15): the two panes and the bar between them measured as one pane by
//! `output::whole_probe`, never the focused pane's own rows. The zoom's own resize
//! (ui/zoom.rs) shows nothing.
//!
//! Nothing here takes the mouse: the layer has no listener and no `occlude()`, so GPUI
//! gives it no hitbox and a press goes to whatever is under it.

use gpui::{AnyElement, Context, Task, Window, deferred, div, prelude::*, px, relative};

use super::ClayView;
use super::output::WholeView;
use crate::native_gui::core::client::Status;
use crate::sizepopup::{LINGER, ResizeWatch, label};
use crate::zoom::Observed;

/// Drawn over everything else Clay defers (menus and popups are 1 to 3).
const PRIORITY: usize = 10;
/// The label's size, the same at every zoom (style.css `#size-popup`).
const TEXT_SIZE: f32 = 14.0;

pub(super) struct SizePopup {
    watch: ResizeWatch,
    /// The popup is showing. A flag the hide task clears, not a time compared at render:
    /// the task runs on GPUI's clock, which the snapshot harness can advance.
    up: bool,
    /// Takes the popup down `LINGER` after the last resize; replaced (so cancelled) by
    /// every resize before then.
    hide: Option<Task<()>>,
    /// The output as one pane while it is split, as `output::whole_probe` last measured
    /// it; None until then, and after every change of the split.
    pub(super) whole: WholeView,
}

impl SizePopup {
    /// The window as it opens: the size its later reports are compared with.
    pub(super) fn new(window: &Window) -> Self {
        let size = window.viewport_size();
        Self { watch: ResizeWatch::new(f32::from(size.width), f32::from(size.height), window.scale_factor()), up: false, hide: None, whole: WholeView::default() }
    }

    /// The view's 250 ms clock.
    pub(super) fn tick(&mut self) {
        self.watch.tick();
    }
}

impl ClayView {
    /// The platform reported this window's bounds (a resize or a move, ours or the
    /// user's): the zoom learns which, and a resize that isn't the zoom's shows the size.
    pub(super) fn bounds_observed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // While a restart's window is still being put back (keep.rs, X11) the reports are
        // the window manager's and ours, not the user's.
        let own = self.keep_placing() || self.zoom_observe_bounds(window) == Observed::Ours;
        // What a restart keeps (keep.rs): the kept place taken first, then noted.
        self.keep_place(window);
        self.keep_note(window, cx);
        // The content size, already the new one when GPUI calls its bounds observers.
        let size = window.viewport_size();
        if !self.size_popup.watch.observe(f32::from(size.width), f32::from(size.height), window.scale_factor(), own) {
            return;
        }
        self.size_popup.up = true;
        self.size_popup.hide = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(LINGER).await;
            let _ = this.update(cx, |this, cx| {
                this.size_popup.up = false;
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// The popup's text while it is up: the output's columns and rows - split, those of
    /// the one pane it would be (until that is first measured, the focused pane's). None
    /// on the login and certificate screens, which have no output to measure.
    pub(super) fn size_popup_label(&self) -> Option<String> {
        if !self.size_popup.up || matches!(self.session.core.status(), Status::NeedLogin { .. } | Status::CertMismatch(_)) {
            return None;
        }
        let whole = self.split_other().and_then(|_| self.size_popup.whole.get());
        let (cols, rows) = whole.unwrap_or_else(|| {
            let scroll = self.scroll.borrow();
            (scroll.view_cols, scroll.view_rows)
        });
        Some(label(cols, rows))
    }

    /// The popup, centred on the window over everything else.
    pub(super) fn size_popup_layer(&self) -> Option<AnyElement> {
        let text = self.size_popup_label()?;
        let t = &self.chrome.tokens;
        let pill = div()
            .px(px(8.0))
            .py(px(3.0))
            .bg(t.elevated)
            .border_1()
            .border_color(t.border)
            .rounded(px(4.0))
            .font_family(self.fonts.primary.clone())
            .text_size(px(TEXT_SIZE))
            .line_height(relative(1.2))
            .text_color(t.fg)
            .whitespace_nowrap()
            .child(text);
        Some(deferred(div().absolute().inset_0().flex().items_center().justify_center().child(pill)).with_priority(PRIORITY).into_any_element())
    }
}
