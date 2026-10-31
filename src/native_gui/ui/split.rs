//! `/split` (user request 2026-10-30): the output in two panes, one above the other, each
//! showing a world, with a bar between them for the top pane (its connection ball and name).
//!
//! The focused pane's world is the core's `current` and its view state is the window's own
//! fields (`scroll`, `select`, `shown_world`, ...), so everything that acts on the current
//! world - the typed line, keys, `/world`, paging, the status bar - acts on the focused
//! pane unchanged. The other pane keeps its own copy of those fields here; moving the focus
//! (Shift+Tab, a plain click in the other pane) swaps the two, and drawing or handling the
//! mouse in the other pane swaps them in for the while (`in_other_pane`).

use gpui::{Context, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ScrollWheelEvent, px};

use super::ClayView;
use super::output::ScrollHandle;
use super::select::SelectState;

/// A split as a relaunch keeps it (`/reload`, `/update`): the other pane's world, by name,
/// and whether it is the top one.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub(super) struct KeptSplit {
    pub world: String,
    pub on_top: bool,
}

/// The pane without the focus.
#[derive(Default)]
pub(super) struct OtherPane {
    /// Its world, by name (indices move as worlds come and go).
    pub world: String,
    /// It is the top pane (else the bottom one).
    pub on_top: bool,
    scroll: ScrollHandle,
    select: SelectState,
    shown_world: Option<usize>,
    history_check: bool,
    textdiv_visit: Option<(Option<u64>, String)>,
}

impl ClayView {
    /// The other pane's world, while split and while the world is there.
    pub(super) fn split_other(&self) -> Option<usize> {
        let pane = self.split.as_ref()?;
        self.session.core.worlds.iter().position(|w| w.name == pane.world)
    }

    /// The other pane's size, for the server (`OtherViews.dims`): rows and columns.
    pub(super) fn split_other_dims(&self) -> Option<crate::websocket::ViewDims> {
        let world = self.split_other()?;
        let s = self.split.as_ref()?.scroll.borrow();
        (s.view_rows > 0).then_some(crate::websocket::ViewDims { world, lines: s.view_rows, columns: s.view_cols })
    }

    /// `/split [world]`: a pane opens above showing `world` - else the next world in order
    /// after the current one (`util::split_next_world`, the console's rule) - and the focus
    /// stays below. Already split, a named world goes into the pane without the focus.
    /// Returns what to tell the user.
    pub(super) fn split_open(&mut self, world: Option<&str>) -> String {
        let core = &self.session.core;
        let named = match world.map(str::trim).filter(|w| !w.is_empty()) {
            Some(name) => match core.worlds.iter().position(|w| w.name.eq_ignore_ascii_case(name)) {
                Some(i) => Some(i),
                None => return format!("World '{name}' not found."),
            },
            None => None,
        };
        let target = match (named, self.split.is_some()) {
            (Some(i), _) => i,
            (None, true) => return "The output is already split; /unsplit closes the pane without the focus.".to_string(),
            (None, false) => {
                let worlds: Vec<(&str, bool)> = core.worlds.iter().map(|w| (w.name.as_str(), w.connected)).collect();
                match crate::util::split_next_world(&worlds, core.current) {
                    Some(i) => i,
                    None => return "There is no other world to show.".to_string(),
                }
            }
        };
        let name = core.worlds[target].name.clone();
        let reply = match self.split.as_mut() {
            Some(pane) => {
                pane.world = name.clone();
                pane.shown_world = None;
                format!("The other pane shows {name}.")
            }
            None => {
                self.split = Some(OtherPane { world: name.clone(), on_top: true, ..Default::default() });
                format!("Split: {name} is shown above. Shift+Tab moves between the panes; /unsplit closes the one without the focus.")
            }
        };
        self.split_changed();
        reply
    }

    /// `/split [world]` or `/unsplit`, typed here or sent back by the server (an action):
    /// what it says goes in the output.
    pub(super) fn split_command(&mut self, command: &str, cx: &mut Context<Self>) {
        let mut words = command.split_whitespace();
        let first = words.next().unwrap_or("").to_lowercase();
        let rest: Vec<&str> = words.collect();
        let reply = if first == "/unsplit" {
            self.split_close()
        } else {
            let world = (!rest.is_empty()).then(|| rest.join(" "));
            self.split_open(world.as_deref())
        };
        self.client_line(&reply, cx);
        self.report_other_views(cx);
        cx.notify();
    }

    /// `/unsplit`: the pane without the focus closes.
    pub(super) fn split_close(&mut self) -> String {
        let Some(pane) = self.split.take() else { return "The output isn't split.".to_string() };
        if let Some(m) = self.more.get_mut(&pane.world).filter(|_| self.session.core.current_world().is_none_or(|w| w.name != pane.world)) {
            m.left();
        }
        self.split_changed();
        format!("Closed the pane showing {}.", pane.world)
    }

    /// The split changed: the core skips the other pane's world when switching, and the
    /// server hears of it on the next report (`report_other_views`).
    pub(super) fn split_changed(&mut self) {
        self.session.core.split_skip = self.split_other();
    }

    /// Shift+Tab (`split_focus`), or a plain click in the other pane: the focus moves there.
    /// Its world becomes the current one; neither pane's world changes, so it is no switch.
    pub(super) fn split_focus_swap(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.split_other() else { return };
        let Some(mut pane) = self.split.take() else { return };
        let left = self.session.core.current;
        let left_name = self.session.core.current_world().map(|w| w.name.clone()).unwrap_or_default();
        self.swap_pane_state(&mut pane);
        pane.world = left_name;
        pane.on_top = !pane.on_top;
        self.split = Some(pane);
        self.session.core.split_skip = Some(left);
        // The worlds on screen besides this one now: the pane left, and the followers'.
        let (others, _) = self.other_views_now(cx);
        let fx = self.session.core.focus_world(index, others);
        self.after(fx, cx);
        cx.notify();
    }

    fn swap_pane_state(&mut self, pane: &mut OtherPane) {
        std::mem::swap(&mut self.scroll, &mut pane.scroll);
        std::mem::swap(&mut self.select, &mut pane.select);
        std::mem::swap(&mut self.shown_world, &mut pane.shown_world);
        std::mem::swap(&mut self.history_check, &mut pane.history_check);
        std::mem::swap(&mut self.textdiv.visit, &mut pane.textdiv_visit);
    }

    /// Run `f` as if the other pane had the focus - its world current, its view state in
    /// the window's fields - and put everything back after: how the other pane is drawn
    /// and how its mouse is handled, with the code the focused pane uses. None while not
    /// split.
    pub(super) fn in_other_pane<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> Option<R> {
        let index = self.split_other()?;
        let mut pane = self.split.take()?;
        let current = self.session.core.current;
        self.swap_pane_state(&mut pane);
        self.session.core.current = index;
        self.drawing_other_pane = true;
        let r = f(self);
        self.drawing_other_pane = false;
        self.session.core.current = current;
        self.swap_pane_state(&mut pane);
        self.split = Some(pane);
        Some(r)
    }

    /// The split, for a relaunch to bring back.
    pub(super) fn split_to_keep(&self) -> Option<KeptSplit> {
        self.split.as_ref().map(|p| KeptSplit { world: p.world.clone(), on_top: p.on_top })
    }

    /// A relaunch's split, before the worlds have arrived: its pane finds its world by name
    /// once they do.
    pub(super) fn split_restore(&mut self, kept: KeptSplit) {
        self.split = Some(OtherPane { world: kept.world, on_top: kept.on_top, ..Default::default() });
    }

    /// Before each frame's output: the other pane's world switched, or went (the split
    /// closes then), and its more-mode budget is its own height.
    pub(super) fn follow_other_pane(&mut self) {
        let Some(pane) = self.split.as_ref() else { return };
        // Before the worlds arrive (a relaunch), there is nothing to look its world up in.
        if self.session.core.worlds.is_empty() {
            return;
        }
        let Some(index) = self.split_other() else {
            // Its world was removed: the split closes.
            self.split = None;
            self.split_changed();
            return;
        };
        if pane.shown_world != Some(index) {
            let name = pane.world.clone();
            self.reveal_shown.insert(name, std::time::Instant::now());
            if let Some(pane) = self.split.as_mut() {
                pane.shown_world = Some(index);
                pane.scroll.borrow_mut().follow_bottom();
            }
            self.split_changed();
        }
    }

    /// The rows and columns more-mode pages `world` in: the other pane's when it shows that
    /// world (and the focused pane doesn't), else the focused pane's.
    pub(super) fn pane_view_for(&self, world: &str) -> (usize, usize) {
        let focused = self.session.core.current_world().is_some_and(|w| w.name == world);
        let s = match self.split.as_ref().filter(|p| p.world == world && !focused) {
            Some(pane) => pane.scroll.borrow(),
            None => self.scroll.borrow(),
        };
        (s.view_rows, s.view_cols)
    }

    /// The other pane's world paused: its view goes to the bottom, where the held lines
    /// wait (the focused pane's does this in `follow_world`).
    pub(super) fn other_pane_paused(&mut self, world: &str) {
        if let Some(pane) = self.split.as_ref().filter(|p| p.world == world) {
            pane.scroll.borrow_mut().follow_bottom();
        }
    }

    // ---- the other pane's mouse ----

    pub(super) fn other_pane_mouse_down(&mut self, ev: &MouseDownEvent, cx: &mut Context<Self>) {
        let (x, y) = (f32::from(ev.position.x), f32::from(ev.position.y));
        let on_bar = self.in_other_pane(|this| this.scroll.borrow().bar_at(x, y).is_some()).unwrap_or(false);
        if ev.button == MouseButton::Right || on_bar {
            // A right-click (its menu acts on this pane) or the scroll bar (the window
            // carries the thumb's drag): the pane takes the focus first.
            self.split_focus_swap(cx);
            self.output_mouse_down(ev, cx);
            return;
        }
        // Text is selected and links followed here without moving the focus.
        self.in_other_pane(|this| this.output_mouse_down(ev, cx));
    }

    pub(super) fn other_pane_mouse_move(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        self.in_other_pane(|this| this.output_mouse_move(ev, cx));
    }

    pub(super) fn other_pane_mouse_up(&mut self, ev: &MouseUpEvent, cx: &mut Context<Self>) {
        // A plain click - no text selected, no link followed - moves the focus here.
        if self.in_other_pane(|this| this.output_mouse_up(ev, cx)).unwrap_or(false) {
            self.split_focus_swap(cx);
        }
    }

    pub(super) fn other_pane_wheel(&mut self, ev: &ScrollWheelEvent, cx: &mut Context<Self>) {
        self.in_other_pane(|this| this.wheel_scroll(ev, cx));
    }

    /// The mouse wheel over the output: scroll it, and past the bottom feed held output in
    /// a row at a time.
    pub(super) fn wheel_scroll(&mut self, ev: &ScrollWheelEvent, cx: &mut Context<Self>) {
        // Positive is towards the user (down the page in GPUI: content moves up).
        let dy = -f32::from(ev.delta.pixel_delta(px(self.font_size)).y);
        let at_bottom = self.scroll.borrow().at_bottom();
        if dy > 0.0 && at_bottom && self.pending_total() > 0 {
            let font_size = self.font_size;
            let more = self.more_mut();
            more.wheel_px += dy;
            let rows = (more.wheel_px / font_size).floor() as usize;
            if rows > 0 {
                more.wheel_px -= rows as f32 * font_size;
                self.release_rows(rows, cx);
            }
        } else {
            self.more_mut().wheel_px = 0.0;
            let rows = (dy / self.font_size).round() as isize;
            if rows != 0 {
                self.user_scroll(rows);
            }
        }
        cx.notify();
    }
}
