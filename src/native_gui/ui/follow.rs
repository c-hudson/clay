//! More windows on the main window's connection: `/window [world]`, `/window --grep` and
//! the menu's New Window. Each is a view of its own (world on screen, scroll, more-mode,
//! input) over a follower session (`Session::follower`), not another client: the main
//! window hands it every message the server sends, and sends what it asks for.
//!
//! The server sees one client, so a reply has no address. It goes to the window that
//! asked last: the main window counts what it sends (`Session::requests_sent`), and a
//! reply goes to the follower that relayed the latest request, while nothing else has
//! been asked since and for a little while after. Anything else reaches every window,
//! except what serves the main window's own windows (the editors, a new window, an MCP
//! edit, a MUD's changed certificate).
//!
//! For the same reason the server knows of one world on screen, the main window's: it
//! tells it which worlds its followers show (`report_other_views`, `OtherViews`), so a
//! world up in a follower is seen there, not left unseen for Unseen First to return to.

use std::time::{Duration, Instant};

use gpui::{AnyWindowHandle, Context, WeakEntity};

use super::ClayView;
use super::viewmode::ViewMode;
use crate::native_gui::core::client::{Input as NetInput, PendingRequest};
use crate::native_gui::core::session::Session;
use crate::websocket::WsMessage;

/// How long a reply can be a follower's after it asked.
const REPLY_WINDOW: Duration = Duration::from_secs(10);

#[derive(Default)]
pub struct Link {
    /// The main window's: the windows on its connection.
    followers: Vec<(WeakEntity<ClayView>, AnyWindowHandle)>,
    /// A follower's: the window whose connection it shares.
    pub leader: Option<WeakEntity<ClayView>>,
    /// This view's own window (where a `/window` opened from it cascades from).
    pub own_window: Option<AnyWindowHandle>,
    /// The main window's: the follower that asked last, the request count then, and when.
    reply_to: Option<(WeakEntity<ClayView>, u64, Instant)>,
}

impl Link {
    #[cfg(feature = "native-gui-snapshot")]
    pub fn follower_window(&self, n: usize) -> Option<AnyWindowHandle> {
        self.followers.iter().filter(|(f, _)| f.upgrade().is_some()).nth(n).map(|(_, h)| *h)
    }

    #[cfg(feature = "native-gui-snapshot")]
    pub fn follower_views(&self) -> Vec<WeakEntity<ClayView>> {
        self.followers.iter().filter(|(f, _)| f.upgrade().is_some()).map(|(f, _)| f.clone()).collect()
    }
}

/// A reply to one client's request: it goes to the window that asked.
fn is_reply(msg: &WsMessage) -> bool {
    match msg {
        WsMessage::ExecuteLocalCommand { .. }
        | WsMessage::CalculatedWorld { .. }
        | WsMessage::WorldSwitchResult { .. }
        | WsMessage::SetInputBuffer { .. }
        | WsMessage::ConnectionsListResponse { .. }
        | WsMessage::ImportResult { .. }
        | WsMessage::ImportNeedsInsecureConfirm { .. }
        | WsMessage::PasswordChanged { .. }
        | WsMessage::WorldCreated { .. }
        | WsMessage::ChatLookupResult { .. }
        | WsMessage::BanListResponse { .. }
        | WsMessage::UnbanResult { .. }
        | WsMessage::AnsiMusic { .. } => true,
        // This client's own notices (`/version`, a command's complaint): outside the
        // world's numbered output.
        WsMessage::ServerData { from_server: false, seq: 0, end_seq: None, .. } => true,
        _ => false,
    }
}

/// For the main window's own windows (the editors, a new window, an MCP edit, a MUD's
/// changed certificate): the main window alone.
fn is_leaders(msg: &WsMessage) -> bool {
    matches!(
        msg,
        WsMessage::OpenWindow { .. }
            | WsMessage::McpEditOpen { .. }
            | WsMessage::FileEditorState { .. }
            | WsMessage::FileSaved { .. }
            | WsMessage::OpenImport { .. }
            | WsMessage::CertMismatch { .. }
            | WsMessage::ThemeEditorState { .. }
            | WsMessage::ThemeFileSaved { .. }
            | WsMessage::KeybindEditorState { .. }
            | WsMessage::KeybindFileSaved { .. }
            | WsMessage::ActionEditorState { .. }
            | WsMessage::NoteEditorState { .. }
    )
}

impl ClayView {
    pub(super) fn is_follower(&self) -> bool {
        self.link.leader.is_some()
    }

    /// What the connection reports. In the main window: kept, and passed on to the windows
    /// that follow it (a reply only to the one that asked).
    pub(super) fn handle(&mut self, input: NetInput, cx: &mut Context<Self>) {
        let msg = match &input {
            NetInput::Message(m) => Some(&**m),
            _ => None,
        };
        if msg.is_some_and(is_reply) {
            if let Some((f, _, _)) = self.link.reply_to.take_if_current(self.session.requests_sent) {
                if f.upgrade().is_some() {
                    self.forward(input, None, vec![f], cx);
                    return;
                }
            }
        }
        // The kind of scrollback a follower is about to be handed, so it lands the same way.
        let request = match msg {
            Some(WsMessage::ScrollbackLines { request_id: Some(id), .. }) => self.session.core.pending_request(*id).map(|r| (*id, r)),
            _ => None,
        };
        self.link.followers.retain(|(f, _)| f.upgrade().is_some());
        let shared = !msg.is_some_and(|m| is_reply(m) || is_leaders(m));
        let copy = (shared && !self.link.followers.is_empty()).then(|| input.clone());
        let fx = self.session.handle(input);
        self.after(fx, cx);
        if let Some(input) = copy {
            let to = self.link.followers.iter().map(|(f, _)| f.clone()).collect();
            self.forward(input, request, to, cx);
        }
    }

    /// Hand `input` to followers, once this update is over (each one's handling may reach
    /// back to this window).
    fn forward(&mut self, input: NetInput, request: Option<(u64, PendingRequest)>, to: Vec<WeakEntity<ClayView>>, cx: &mut Context<Self>) {
        cx.defer(move |cx| {
            for f in to {
                let input = input.clone();
                let _ = f.update(cx, |v, cx| v.follow(input, request, cx));
            }
        });
    }

    /// A follower takes what its leader received.
    fn follow(&mut self, input: NetInput, request: Option<(u64, PendingRequest)>, cx: &mut Context<Self>) {
        if let Some((id, r)) = request {
            self.session.core.adopt_request(id, r);
        }
        let fx = self.session.handle(input);
        self.after(fx, cx);
    }

    /// A follower's: what the user's doing here asked to send, for the leader to send.
    pub(super) fn relay_outbox(&mut self, cx: &mut Context<Self>) {
        let Some(leader) = self.link.leader.clone() else { return };
        let msgs = self.session.take_outbox();
        if msgs.is_empty() {
            return;
        }
        let me = cx.entity().downgrade();
        cx.defer(move |cx| {
            let _ = leader.update(cx, |v, cx| v.relay(me, msgs, cx));
        });
    }

    /// The leader sends a follower's messages; replies are the follower's for now.
    fn relay(&mut self, from: WeakEntity<ClayView>, msgs: Vec<WsMessage>, cx: &mut Context<Self>) {
        for m in msgs {
            let fx = self.session.send(m);
            self.after(fx, cx);
        }
        self.link.reply_to = Some((from, self.session.requests_sent, Instant::now()));
        // A follower that switched worlds says so with its RequestWorldState: what's on
        // screen in the windows changed.
        self.report_other_views(cx);
    }

    /// The main window's: tell the server which worlds the windows following it show
    /// (`Session::set_other_views`; sent only when that changes). They say nothing for
    /// themselves, so without it a world shown in one was unseen to the server for good.
    /// Also from the clock, for a window opened or closed.
    pub(super) fn report_other_views(&mut self, cx: &mut Context<Self>) {
        if self.is_follower() {
            return;
        }
        self.link.followers.retain(|(f, _)| f.upgrade().is_some());
        let (worlds, dims) = self.other_views_now(cx);
        let fx = self.session.set_other_views(worlds, dims);
        if !fx.is_empty() {
            self.after(fx, cx);
        }
    }

    /// The worlds on screen besides this window's focused one - its split's other pane
    /// (`/split`), each window following it and their split panes - with the size each is
    /// shown at (`OtherViews.dims`).
    pub(super) fn other_views_now(&self, cx: &Context<Self>) -> (Vec<usize>, Vec<crate::websocket::ViewDims>) {
        let mut worlds: Vec<usize> = self.split_other().into_iter().collect();
        let mut dims: Vec<crate::websocket::ViewDims> = self.split_other_dims().into_iter().collect();
        for f in self.link.followers.iter().filter_map(|(f, _)| f.upgrade()) {
            let view = f.read(cx);
            let core = &view.session.core;
            if core.worlds.is_empty() {
                continue;
            }
            worlds.push(core.current);
            let s = view.scroll.borrow();
            if s.view_rows > 0 {
                dims.push(crate::websocket::ViewDims { world: core.current, lines: s.view_rows, columns: s.view_cols });
            }
            worlds.extend(view.split_other());
            dims.extend(view.split_other_dims());
        }
        (worlds, dims)
    }

    /// Open a window on this connection (the main window's, whichever window asks).
    pub(super) fn open_view(&mut self, mode: ViewMode, title: String, size: (f32, f32), cx: &mut Context<Self>) {
        if let Some(leader) = self.link.leader.clone() {
            cx.defer(move |cx| {
                let _ = leader.update(cx, |v, cx| v.open_view(mode, title, size, cx));
            });
            return;
        }
        let me = cx.entity().downgrade();
        // Copied now: whatever arrives after this reaches the new window too.
        let session = Session::follower(&self.session);
        let fonts = self.fonts.clone();
        cx.defer(move |cx| {
            // Down and right of this window, a step further for each window already open
            // from it, rather than centred on the screen exactly over it.
            let at = me.upgrade().map(|v| {
                let v = v.read(cx);
                (v.link.own_window, v.link.followers.iter().filter(|(f, _)| f.upgrade().is_some()).count())
            });
            let origin = at.and_then(|(own, open)| {
                let step = gpui::px(32.0 * ((open % 8) + 1) as f32);
                own?.update(cx, |_, window, _| window.bounds().origin).ok().map(|o| gpui::point(o.x + step, o.y + step))
            });
            let how = super::ViewWindow { title: title.into(), size, is_main: false, leader: Some(me.clone()), origin };
            if let Some(opened) = super::open_view_window(cx, fonts, session, None, mode, how) {
                let _ = me.update(cx, |v, _| v.link.followers.push(opened));
            }
        });
    }

    /// Redraw this window and the ones that follow it (a Discord emoji arrived).
    pub(super) fn repaint_all(&mut self, cx: &mut Context<Self>) {
        cx.notify();
        for (f, _) in &self.link.followers {
            let _ = f.update(cx, |_, cx| cx.notify());
        }
    }

    /// The window to do window things in (Settings, Help, the editors...): the main one.
    pub(super) fn owner(&self, cx: &mut Context<Self>) -> WeakEntity<ClayView> {
        self.link.leader.clone().unwrap_or_else(|| cx.entity().downgrade())
    }
}

trait TakeIfCurrent {
    fn take_if_current(&mut self, sent: u64) -> Option<(WeakEntity<ClayView>, u64, Instant)>;
}

impl TakeIfCurrent for Option<(WeakEntity<ClayView>, u64, Instant)> {
    /// The follower a reply is for: nothing asked since, and not long ago. It stays the
    /// follower's (a command can have several replies) until either is no longer true.
    fn take_if_current(&mut self, sent: u64) -> Option<(WeakEntity<ClayView>, u64, Instant)> {
        match self {
            Some((_, mark, at)) if *mark == sent && at.elapsed() < REPLY_WINDOW => self.clone(),
            _ => {
                *self = None;
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(seq: u64, from_server: bool) -> WsMessage {
        WsMessage::ServerData {
            archive_sourced: false,
            world_index: 0,
            data: "x".into(),
            is_viewed: false,
            ts: 0,
            from_server,
            seq,
            end_seq: None,
            flush: false,
            gagged: false,
            highlight_colors: Vec::new(),
        }
    }

    #[test]
    fn replies_and_the_main_windows_own_are_told_apart_from_the_rest() {
        assert!(is_reply(&WsMessage::ExecuteLocalCommand { command: "/help".into() }));
        assert!(is_reply(&WsMessage::CalculatedWorld { index: Some(1) }));
        assert!(is_reply(&data(0, false)), "a client notice");
        assert!(!is_reply(&data(7, false)), "numbered output is everyone's");
        assert!(!is_reply(&data(0, true)), "the MUD's text is everyone's");
        assert!(is_leaders(&WsMessage::OpenWindow { world: None }));
        assert!(!is_leaders(&WsMessage::ActivityUpdate { count: 1 }) && !is_reply(&WsMessage::ActivityUpdate { count: 1 }));
    }
}
