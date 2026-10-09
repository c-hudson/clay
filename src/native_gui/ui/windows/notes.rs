//! A world's notes (app.js `openNoteEditor`, `enterNoteMode`): an OS window of its own
//! with a text area, Save and Cancel. And the MCP simpleedit editor (`enterMcpEditMode`):
//! the same editor over the main window, for text the MUD sends to be edited and sent
//! back.

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

/// How long "Saved" or "Sent" shows.
const STATUS_FOR: Duration = Duration::from_millis(1500);

/// What the editor is for: a world's notes, or a MUD's simpleedit session.
#[derive(Clone, Debug)]
pub enum Purpose {
    Notes { world_index: usize },
    Mcp { world_index: usize, reference: String, edit_type: String, name: String },
}

pub struct NoteView {
    main: WeakEntity<ClayView>,
    purpose: Purpose,
    title: String,
    text: Entity<TextareaState>,
    status_at: Option<Instant>,
    /// Another simpleedit arrived while this one was open (and was refused).
    ignored: bool,
}

impl NoteView {
    /// A world's notes: asked for from the server, shown when they come.
    pub fn notes(main: WeakEntity<ClayView>, world_index: usize, name: &str, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let text = cx.new(|cx| TextareaState::new(window, cx));
        let _ = main.update(cx, |v, cx| v.send_message(WsMessage::RequestNoteEditorState { world_index }, cx));
        window.set_window_title(&format!("Clay - Notes: {name}"));
        Self { main, purpose: Purpose::Notes { world_index }, title: "Notes".into(), text, status_at: None, ignored: false }
    }

    /// A simpleedit session, with the MUD's text.
    pub fn mcp(main: WeakEntity<ClayView>, purpose: Purpose, content: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let text = cx.new(|cx| TextareaState::new(window, cx).default_value(content));
        let title = match &purpose {
            Purpose::Mcp { name, .. } => format!("Edit: {name}"),
            Purpose::Notes { .. } => "Notes".into(),
        };
        Self { main, purpose, title, text, status_at: None, ignored: false }
    }

    /// The notes arrived (`NoteEditorState`).
    pub fn set_notes(&mut self, world_name: &str, notes: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.title = format!("Notes: {world_name}");
        window.set_window_title(&format!("Clay - Notes: {world_name}"));
        let notes = notes.to_string();
        self.text.update(cx, |t, cx| t.set_value(notes, window, cx));
        cx.notify();
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
        };
        let _ = self.main.update(cx, |v, cx| v.send_message(msg, cx));
        self.status_at = Some(Instant::now());
        cx.notify();
        // Fade the status out again.
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(STATUS_FOR).await;
            let _ = this.update(cx, |_, cx| cx.notify());
        })
        .detach();
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
        let status = self.status_at.filter(|at| at.elapsed() < STATUS_FOR).map(|_| if mcp { "Sent" } else { "Saved" });
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
                    .child(div().text_size(px(11.0)).text_color(t.success).children(status))
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
