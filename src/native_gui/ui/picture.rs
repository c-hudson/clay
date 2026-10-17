//! Import Picture in the native GUI: Menu > Import Picture (a file dialog) and pictures
//! dropped onto the window. The server uploads them (src/picture.rs); this window sends the
//! files in chunks (`picture::ClientUploads`) and puts each batch's links into its input
//! line. Every window of a connection sees the acks and answers (follow.rs leaves them
//! unclassified); upload ids are process-wide, so a window acts only on its own.

use std::path::PathBuf;
use std::time::Instant;

use gpui::{Context, Window};

use super::ClayView;
use crate::picture::{self, ClientNote};
use crate::websocket::WsMessage;

/// GPUI decides "hovered" by the last input's kind, and a file dragged in from another
/// program never changes it: after a key press, every hitbox counts as not hovered until
/// the mouse moves, and a drop onto the window was silently ignored (a drag from a file
/// manager sends the window no ordinary mouse move). This element, painted in the main view,
/// answers the drag's first move with a real one, so the drop lands.
pub(super) fn drop_after_typing_fix() -> impl gpui::IntoElement {
    use gpui::Styled;
    gpui::canvas(
        |_, _, _| (),
        |_, _, window, _cx| {
            window.on_mouse_event(|ev: &gpui::MouseMoveEvent, phase, window, cx| {
                if phase == gpui::DispatchPhase::Capture && cx.has_active_drag() && window.last_input_was_keyboard() {
                    let position = ev.position;
                    window.defer(cx, move |window, cx| {
                        window.dispatch_event(
                            gpui::PlatformInput::MouseMove(gpui::MouseMoveEvent {
                                position,
                                pressed_button: Some(gpui::MouseButton::Left),
                                modifiers: gpui::Modifiers::default(),
                            }),
                            cx,
                        );
                    });
                }
            });
        },
    )
    .absolute()
    .size_0()
}

impl ClayView {
    /// Why pictures can't be uploaded right now, if they can't.
    fn pictures_unavailable(&self) -> Option<&'static str> {
        if !self.session.core.server_picture_upload {
            return Some("This server can't upload pictures - update it.");
        }
        let off = self.session.core.settings.as_ref().is_some_and(|s| picture::PictureService::from_name(&s.picture_host).is_off());
        off.then_some(picture::OFF_MESSAGE)
    }

    /// Menu > Import Picture: the platform's file dialog.
    pub(super) fn import_picture_menu(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(why) = self.pictures_unavailable() {
            self.client_line(why, cx);
            return;
        }
        let chosen = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Import Picture".into()),
        });
        cx.spawn(async move |this, cx| {
            match chosen.await {
                Ok(Ok(Some(paths))) => {
                    let _ = this.update(cx, |v, cx| v.import_pictures(paths, cx));
                }
                // Linux without xdg-desktop-portal has no file dialog to show.
                Ok(Err(e)) => {
                    let _ = this.update(cx, |v, cx| {
                        v.client_line(&format!("No file dialog is available ({e}); drag the picture onto the window instead."), cx)
                    });
                }
                _ => {}
            }
        })
        .detach();
    }

    /// Pictures dropped onto the window or chosen in the dialog: one batch, read off the UI
    /// thread.
    pub(super) fn import_pictures(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        if paths.is_empty() {
            return;
        }
        if let Some(why) = self.pictures_unavailable() {
            self.client_line(why, cx);
            return;
        }
        let read = cx.background_executor().spawn(async move {
            paths.iter().map(|p| (picture::display_name(p), picture::read_picture(p))).collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let results = read.await;
            let _ = this.update(cx, |v, cx| {
                let mut pictures = Vec::new();
                for (name, result) in results {
                    match result {
                        Ok((bytes, _)) => pictures.push((name, bytes)),
                        Err(e) => v.client_line(&e, cx),
                    }
                }
                let (msgs, notes) = v.pictures.add_batch(pictures, Instant::now());
                v.picture_progress(msgs, notes, cx);
            });
        })
        .detach();
    }

    /// The server's `PictureUploadAck` / `PictureUploaded` (each window gets them; a
    /// window acts only on its own uploads).
    pub(super) fn picture_message(&mut self, msg: WsMessage, cx: &mut Context<Self>) {
        let now = Instant::now();
        match msg {
            WsMessage::PictureUploadAck { id, received } => {
                let msgs = self.pictures.on_ack(id, received, now);
                self.picture_progress(msgs, Vec::new(), cx);
            }
            WsMessage::PictureUploaded { id, url, error } => {
                let (msgs, notes, _) = self.pictures.on_uploaded(id, url, error, now);
                self.picture_progress(msgs, notes, cx);
            }
            _ => {}
        }
    }

    /// The clock: a lost ack is resent, a server that never answers given up on.
    pub(super) fn picture_tick(&mut self, cx: &mut Context<Self>) {
        if !self.pictures.is_busy() {
            return;
        }
        let (msgs, notes) = self.pictures.tick(Instant::now());
        self.picture_progress(msgs, notes, cx);
    }

    /// The connection went: this window's uploads with it.
    pub(super) fn pictures_connection_lost(&mut self, cx: &mut Context<Self>) {
        let notes = self.pictures.connection_lost();
        self.picture_progress(Vec::new(), notes, cx);
    }

    fn picture_progress(&mut self, msgs: Vec<WsMessage>, notes: Vec<ClientNote>, cx: &mut Context<Self>) {
        for msg in msgs {
            let fx = self.session.send(msg);
            self.after(fx, cx);
        }
        for note in notes {
            match note {
                ClientNote::Line(line) => self.client_line(&line, cx),
                ClientNote::Insert(links) => self.insert_picture_links(&links, cx),
            }
        }
    }

    /// A batch's links into the input line by the one rule (`Editor::insert_link`), or
    /// pasted into a live shell world.
    fn insert_picture_links(&mut self, links: &str, cx: &mut Context<Self>) {
        if self.session.core.current_is_live_shell() {
            let fx = self.session.core.shell_paste(links.to_string());
            let rest = self.session.apply(fx);
            self.after(rest, cx);
            return;
        }
        self.editor.insert_link(links);
        self.input_touched(cx);
        cx.notify();
    }
}
