//! Screen readers in the native GUI (docs/markdown/24-accessibility.md). GPUI exposes its
//! elements through AccessKit (AT-SPI on Linux, UI Automation on Windows, NSAccessibility on
//! macOS) once a screen reader is running; an element is in that tree when it has an id and
//! a role. Clay's output and input are painted, so they say what they hold here:
//!
//! - the window's root is a named group, and the input claims the screen reader's focus
//!   while the root holds the keyboard's (`aria_active_descendant`), with the line being
//!   typed as its value;
//! - the output is a log whose children are the newest lines, as plain text;
//! - in Screen Reader mode (this GUI's own setting) a polite live region reads each line out
//!   once, as it is shown: an arrival, or a line more-mode lets through - never a redraw, a
//!   scroll or a world switch.

use std::collections::HashSet;

use gpui::accesskit::{Live, Node, Role};
use gpui::{div, prelude::*, px, Context, Window};

use super::ClayView;

/// How many of the newest lines the output's node lists for a screen reader to review.
const REVIEW_LINES: usize = 50;

/// A line as a screen reader should hear it: no escape codes, no MUD tag, no line breaks.
pub(super) fn spoken(text: &str) -> String {
    crate::util::strip_mud_tag(&super::find::plain(text)).trim().to_string()
}

impl ClayView {
    /// This window's own Screen Reader mode (Settings > General; off until it chose one).
    pub(super) fn screen_reader_on(&self) -> bool {
        self.session.core.settings.as_ref().is_some_and(|s| s.screen_reader_for(crate::font_profile::native_key()))
    }

    /// Lines just shown (`reveal_stamp`'s): in Screen Reader mode the current world's are
    /// read out, once - the live region's text becomes them.
    pub(super) fn sr_note_shown(&mut self, ids: &[u64]) {
        if ids.is_empty() || !self.screen_reader_on() {
            return;
        }
        let Some(world) = self.session.core.current_world() else { return };
        let ids: HashSet<u64> = ids.iter().copied().collect();
        let text: Vec<String> = world
            .lines
            .iter()
            .filter(|l| ids.contains(&l.id) && !l.gagged)
            .map(|l| spoken(&l.text))
            .filter(|t| !t.is_empty())
            .collect();
        if text.is_empty() {
            return;
        }
        self.a11y.announce = text.join("\n");
        self.a11y.announced += 1;
    }

    /// A screen reader started and this window never chose its mode: turn it on, once, and
    /// keep it so (the user can turn it off in Settings), as the Android app does with
    /// TalkBack.
    pub(super) fn sr_detect(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.a11y.detected || !window.is_a11y_active() {
            return;
        }
        let key = crate::font_profile::native_key();
        let Some(s) = self.session.core.settings.as_ref() else { return };
        self.a11y.detected = true;
        let unchosen = s.screen_reader_profiles.as_ref().is_some_and(|p| !p.contains_key(key));
        if unchosen {
            let mut s = s.clone();
            s.set_screen_reader_for(key, true);
            let msg = crate::native_gui::core::client::settings_update(&s);
            self.send_message(msg, cx);
        }
    }

    /// The live region Screen Reader mode reads from: off the screen, its text the lines
    /// last shown. Nothing without the mode, or without a screen reader running.
    pub(super) fn sr_announcer(&self, window: &Window) -> Option<gpui::AnyElement> {
        // Not until there is something to read: a live region that appears empty is
        // announced as nothing.
        if !window.is_a11y_active() || !self.screen_reader_on() || self.a11y.announce.is_empty() {
            return None;
        }
        let text = self.a11y.announce.clone();
        // A counter in the description, so the same text twice is still a change.
        let seq = self.a11y.announced.to_string();
        Some(
            div()
                .id("sr-announcer")
                .absolute()
                .size(px(0.0))
                .overflow_hidden()
                .role(Role::Status)
                .aria_label(text.clone())
                .aria_value(text)
                .aria_description(seq)
                .a11y_synthetic_children(|b| b.parent_node().set_live(Live::Polite))
                .into_any_element(),
        )
    }

    /// The newest lines of the world shown, for the output's node: a screen reader reviews
    /// them as a list of lines. Built only while one is running.
    pub(super) fn review_lines(&self, window: &Window) -> Vec<String> {
        if !window.is_a11y_active() {
            return Vec::new();
        }
        let Some(world) = self.session.core.current_world() else { return Vec::new() };
        let mut out: Vec<String> = world.lines.iter().rev().filter(|l| !l.gagged).map(|l| spoken(&l.text)).filter(|t| !t.is_empty()).take(REVIEW_LINES).collect();
        out.reverse();
        out
    }
}

/// The output's lines as synthetic label nodes under its log node.
pub(super) fn add_review_lines(b: &mut gpui::A11ySubtreeBuilder, lines: Vec<String>) {
    for (i, text) in lines.into_iter().enumerate() {
        let mut node = Node::new(Role::Label);
        node.set_value(text);
        let id = b.synthetic_node_id(("line", i));
        b.push_child(id, node);
    }
}

/// What the screen-reader side keeps between frames.
#[derive(Default)]
pub(super) struct A11yState {
    /// The live region's text: the lines last shown, in Screen Reader mode.
    pub announce: String,
    pub announced: u64,
    /// A running screen reader was noticed (and acted on) this session.
    pub detected: bool,
    /// The newest lines, plain, for the output's node (built each frame while one runs).
    pub review: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_spoken_as_its_text_alone() {
        assert_eq!(spoken("\x1b[1;31m[ooc:] Bob\x1b[0m: hi\r\n"), "Bob: hi");
        assert_eq!(spoken("   "), "");
    }

    #[test]
    fn review_lines_become_label_nodes() {
        // The node builder works on a real tree: check the nodes a line becomes.
        let mut node = Node::new(Role::Label);
        node.set_value("The wind blows.");
        assert_eq!(node.role(), Role::Label);
        assert_eq!(node.value(), Some("The wind blows."));
    }
}
