//! A world's notes (app.js `openNoteEditor`, `enterNoteMode`): an OS window of its own
//! with a text area, Save and Cancel; Save saves and closes it, as in every interface.
//! And the MCP simpleedit editor (`enterMcpEditMode`): the same editor over the main
//! window, for text the MUD sends to be edited and sent back; and `/note <file>`
//! (`enterFileEditMode`): a file on the main instance's machine, which the server sent.

use std::time::{Duration, Instant};

use gpui::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, WeakEntity, Window, div, px, relative,
};
use gpui_component::input::{Textarea, TextareaState};

use crate::native_gui::ui::ClayView;
use crate::native_gui::ui::dialogs::DialogEvent;
use crate::native_gui::ui::kit::Kit;
use crate::native_gui::ui::output::hsla;
use crate::websocket::WsMessage;

/// How long "Not connected - not saved/sent" shows.
const STATUS_FOR: Duration = Duration::from_millis(1500);

/// What the editor is for: a world's notes, a MUD's simpleedit session, or a file on the
/// main instance's machine (`/note <file>`).
#[derive(Clone, Debug)]
pub enum Purpose {
    Notes { world_index: usize },
    Mcp { world_index: usize, reference: String, edit_type: String, name: String },
    File { path: String },
}

pub struct NoteView {
    main: WeakEntity<ClayView>,
    purpose: Purpose,
    title: String,
    text: Entity<TextareaState>,
    status_at: Option<Instant>,
    /// The last Save found no connection: nothing was sent, and the window stays open.
    not_sent: bool,
    /// Another simpleedit arrived while this one was open (and was refused).
    ignored: bool,
    /// A file's save under way, or why it failed: (text, is an error).
    file_status: Option<(String, bool)>,
}

impl NoteView {
    /// A world's notes: asked for from the server, shown when they come.
    pub fn notes(main: WeakEntity<ClayView>, world_index: usize, name: &str, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let text = cx.new(|cx| TextareaState::new(window, cx));
        let _ = main.update(cx, |v, cx| v.send_message(WsMessage::RequestNoteEditorState { world_index }, cx));
        window.set_window_title(&format!("Clay - Notes: {name}"));
        Self { main, purpose: Purpose::Notes { world_index }, title: "Notes".into(), text, status_at: None, not_sent: false, ignored: false, file_status: None }
    }

    /// A simpleedit session, with the MUD's text.
    pub fn mcp(main: WeakEntity<ClayView>, purpose: Purpose, content: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let text = cx.new(|cx| TextareaState::new(window, cx).default_value(content));
        let title = match &purpose {
            Purpose::Mcp { name, .. } => format!("Edit: {name}"),
            Purpose::File { path } => format!("Edit: {}", std::path::Path::new(path).file_name().map_or(path.clone(), |n| n.to_string_lossy().into_owned())),
            Purpose::Notes { .. } => "Notes".into(),
        };
        Self { main, purpose, title, text, status_at: None, not_sent: false, ignored: false, file_status: None }
    }

    /// The notes arrived (`NoteEditorState`).
    pub fn set_notes(&mut self, world_name: &str, notes: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.title = format!("Notes: {world_name}");
        window.set_window_title(&format!("Clay - Notes: {world_name}"));
        let notes = notes.to_string();
        self.text.update(cx, |t, cx| t.set_value(notes, window, cx));
        cx.notify();
    }

    /// The server's answer to a file's Save (`FileSaved`): saved, it closes.
    pub fn file_saved(&mut self, ok: bool, error: Option<&str>, cx: &mut Context<Self>) {
        if ok {
            cx.emit(DialogEvent::Close);
        } else {
            self.file_status = Some((format!("Failed to save: {}", error.unwrap_or("")), true));
            cx.notify();
        }
    }

    /// The file this editor is for, if it is one.
    pub fn file_path(&self) -> Option<&str> {
        match &self.purpose {
            Purpose::File { path } => Some(path),
            _ => None,
        }
    }

    /// A second simpleedit was refused while this one is open.
    pub fn note_ignored(&mut self, cx: &mut Context<Self>) {
        self.ignored = true;
        cx.notify();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let content = self.text.read(cx).value().to_string();
        let msg = match &self.purpose {
            Purpose::Notes { world_index } => WsMessage::UpdateNote { world_index: *world_index, notes: content },
            Purpose::Mcp { world_index, reference, edit_type, .. } => {
                WsMessage::McpEditSet { world_index: *world_index, reference: reference.clone(), edit_type: edit_type.clone(), content }
            }
            Purpose::File { path } => WsMessage::SaveFile { path: path.clone(), content },
        };
        // The text goes through the main window's connection, which outlives this window:
        // sent, it can close at once (notes and an MCP edit alike, as the console's Ctrl+S
        // does). Offline, nothing would be sent, so it stays open with the edits.
        let online = self.main.upgrade().is_some_and(|m| m.read(cx).session.core.is_online());
        if !online {
            self.not_sent = true;
            self.status_at = Some(Instant::now());
            cx.notify();
            return;
        }
        let _ = self.main.update(cx, |v, cx| v.send_message(msg, cx));
        if matches!(self.purpose, Purpose::File { .. }) {
            // The server writes it: closed when it says it is saved (`file_saved`).
            self.file_status = Some(("Saving...".into(), false));
            cx.notify();
        } else {
            cx.emit(DialogEvent::Close);
        }
    }
}

impl EventEmitter<DialogEvent> for NoteView {}
impl Focusable for NoteView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.text.read(cx).focus_handle(cx)
    }
}

impl Render for NoteView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some((k, bar)): Option<(Kit, gpui::Hsla)> = self.main.upgrade().map(|m| {
            let v = m.read(cx);
            (v.kit(), hsla([v.theme.status_bar_bg.r, v.theme.status_bar_bg.g, v.theme.status_bar_bg.b]))
        }) else {
            return div().into_any_element();
        };
        let t = k.t.clone();
        let mcp = matches!(self.purpose, Purpose::Mcp { .. });
        // A save that went out closes the window, so the only status left is "not sent".
        let not_sent = self.status_at.filter(|at| self.not_sent && at.elapsed() < STATUS_FOR).map(|_| {
            (if mcp { "Not connected - not sent" } else { "Not connected - not saved" }.to_string(), true)
        });
        let (status, is_error) = match not_sent.or_else(|| self.file_status.clone()) {
            Some((text, error)) => (Some(text), error),
            None => (None, false),
        };
        let title = if self.ignored { format!("{} (another edit request was ignored)", self.title) } else { self.title.clone() };
        let (accent, hover) = (t.accent, t.hover);
        div()
            .id("note-editor")
            .size_full()
            .flex()
            .flex_col()
            .bg(k.field_bg)
            .font_family(k.fonts.ui.clone())
            .child(
                div()
                    .h(px(26.0))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .px(px(12.0))
                    .bg(bar)
                    .border_b_1()
                    .border_color(t.border)
                    .child(div().font_weight(FontWeight::BOLD).text_color(t.fg).whitespace_nowrap().overflow_hidden().text_ellipsis().child(title))
                    .child(div().text_size(px(11.0)).text_color(if is_error { t.error } else { t.success }).children(status))
                    .child(div().flex_1())
                    .child(
                        div()
                            .id("note-save")
                            .px(px(16.0))
                            .py(px(3.0))
                            .rounded(px(4.0))
                            .bg(accent)
                            .text_color(t.surface)
                            .text_size(px(11.5))
                            .font_weight(FontWeight::BOLD)
                            .cursor_pointer()
                            .hover(|s| s.opacity(0.9))
                            .child("Save")
                            .on_click(cx.listener(|this, _, _w, cx| this.save(cx))),
                    )
                    .child(
                        div()
                            .id("note-cancel")
                            .px(px(16.0))
                            .py(px(3.0))
                            .rounded(px(4.0))
                            .border_1()
                            .border_color(t.border)
                            .text_color(t.muted)
                            .text_size(px(11.5))
                            .font_weight(FontWeight::BOLD)
                            .cursor_pointer()
                            .hover(move |s| s.bg(hover).text_color(t.fg))
                            .child("Cancel")
                            // Unsaved edits go: the notes window closes, the simpleedit ends.
                            .on_click(cx.listener(|_, _, _w, cx| cx.emit(DialogEvent::Close))),
                    ),
            )
            .child(
                div().flex_1().min_h_0().child(
                    Textarea::new(&self.text)
                        .size_full()
                        .appearance(false)
                        .p(px(12.0))
                        .bg(k.field_bg)
                        .text_color(t.fg)
                        .font_family(k.fonts.primary.clone())
                        .text_size(px(13.0))
                        .line_height(relative(1.4)),
                ),
            )
            .into_any_element()
    }
}
