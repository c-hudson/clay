//! The Info window (`/info`, app.js `renderInfoDialog`): the server's report on a world
//! and on Clay (`crate::info`), then this GUI's own section. A snapshot: `/info` again
//! puts a new report in the same window.

use gpui::{
    App, ClipboardItem, Context, EventEmitter, FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, WeakEntity, Window, div, hsla, prelude::*, px, relative,
};

use crate::info::{report_lines, InfoReport};
use crate::native_gui::ui::ClayView;
use crate::native_gui::ui::chrome::MONO_LH;
use crate::native_gui::ui::dialogs::DialogEvent;
use crate::native_gui::ui::kit::Btn;

pub struct InfoView {
    main: WeakEntity<ClayView>,
    focus: FocusHandle,
    report: InfoReport,
    copied: bool,
}

impl InfoView {
    pub fn new(main: WeakEntity<ClayView>, report: InfoReport, cx: &mut Context<Self>) -> Self {
        Self { main, focus: cx.focus_handle(), report, copied: false }
    }

    /// `/info` again: the new report.
    pub fn set(&mut self, report: InfoReport, cx: &mut Context<Self>) {
        self.report = report;
        self.copied = false;
        cx.notify();
    }
}

impl EventEmitter<DialogEvent> for InfoView {}
impl Focusable for InfoView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for InfoView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(k) = self.main.upgrade().map(|m| m.read(cx).kit()) else { return div().into_any_element() };
        let t = k.t.clone();
        let warn = hsla(40.0 / 360.0, 0.85, if t.light { 0.38 } else { 0.6 }, 1.0);
        let sections = self.report.sections.iter().enumerate().map(|(i, s)| {
            let rows = s.rows.iter().map(|r| {
                let color = match r.tone.as_deref() {
                    Some("good") => t.success,
                    Some("bad") => t.error,
                    Some("warn") => warn,
                    _ => t.fg,
                };
                div()
                    .flex()
                    .gap(px(10.0))
                    .py(px(2.0))
                    .child(div().w(px(150.0)).flex_shrink_0().text_color(t.fg2).child(SharedString::from(r.key.clone())))
                    .child(div().flex_1().min_w_0().text_color(color).child(SharedString::from(r.value.clone())))
            });
            let lines = s.lines.iter().map(|l| {
                div().font_family(k.fonts.primary.clone()).line_height(relative(MONO_LH)).text_size(px(11.5)).text_color(t.fg).child(SharedString::from(l.clone()))
            });
            div()
                .when(i > 0, |d| d.mt(px(6.0)).border_t_1().border_color(t.border_subtle))
                .child(
                    div()
                        .py(px(8.0))
                        .text_size(px(11.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(t.accent)
                        .child(s.title.to_uppercase()),
                )
                .child(div().flex().flex_col().children(rows))
                .when(!s.lines.is_empty(), |d| d.child(div().flex().flex_col().pt(px(4.0)).children(lines)))
        });
        let copy_label = if self.copied { "Copied" } else { "Copy" };
        div()
            .id("info-window")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .p(px(14.0))
            .font_family(k.fonts.ui.clone())
            .text_size(px(13.0))
            .text_color(t.fg)
            .on_key_down(cx.listener(|_, ev: &KeyDownEvent, _w, cx| {
                if matches!(ev.keystroke.key.as_str(), "escape" | "enter") {
                    cx.emit(DialogEvent::Close);
                    cx.stop_propagation();
                }
            }))
            .child(k.header("info-close", self.report.title.clone(), cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))))
            .child(
                div()
                    .id("info-content")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .line_height(relative(1.4))
                    .py(px(4.0))
                    .children(sections),
            )
            .child(
                k.buttons()
                    .child(k.button("info-copy", copy_label, Btn::Plain, Some(Box::new(cx.listener(|this, _, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(report_lines(&this.report, false).join("\n")));
                        this.copied = true;
                        cx.notify();
                    })))))
                    .child(k.button("info-ok", "Ok", Btn::Primary, Some(Box::new(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close)))))),
            )
            .into_any_element()
    }
}
