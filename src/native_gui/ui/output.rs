//! The output area: one custom-painted element.
//!
//! Each line is shaped once with the platform's text system, then wrapped and painted
//! here, so the result follows the WebView's CSS exactly rather than GPUI's own text
//! element:
//!
//! - Rows are exactly one font-size tall (`#output .line { line-height: 1.0 }`), and the
//!   baseline sits where JetBrains Mono's metrics put it, whichever font a glyph came
//!   from.
//! - A coloured run's background covers the font's full height, which is taller than the
//!   row. Rows paint top to bottom, background and text one row at a time, so a row's
//!   background covers the descenders of the row above, as it does in the browser.
//! - Block elements and coloured spaces are exact cells, one column wide and one row
//!   tall, painted after everything else (style.css `.blk`).
//! - The container has 4px padding, and the view follows the bottom.

use std::ops::Range;
use std::sync::Arc;

use gpui::{
    App, Bounds, ContentMask, Font, Hsla, IntoElement, LineLayout, Pixels, Rgba, SharedString, Styled, TextRun,
    UnderlineStyle, Window, canvas, fill, point, px, size,
};
use unicode_segmentation::UnicodeSegmentation;

use super::fonts::{FontClass, Fonts, class_of};
use crate::native_gui::core::style::{Cell, Rgb, StyledLine};
use crate::native_gui::core::wrap::wrap_rows;

/// `#output-container { padding: 4px }`.
const PADDING: f32 = 4.0;

pub fn hsla(c: Rgb) -> Hsla {
    Rgba { r: c[0] as f32 / 255.0, g: c[1] as f32 / 255.0, b: c[2] as f32 / 255.0, a: 1.0 }.into()
}

fn hsla_alpha(c: Rgb, a: f32) -> Hsla {
    Rgba { r: c[0] as f32 / 255.0, g: c[1] as f32 / 255.0, b: c[2] as f32 / 255.0, a }.into()
}

/// Everything the output element needs for one frame.
pub struct OutputSnapshot {
    pub lines: Vec<Arc<StyledLine>>,
    pub background: Rgb,
    pub font_size: f32,
    pub fonts: Fonts,
    /// How far the view is scrolled up from the bottom, in rows.
    pub scroll_rows: usize,
    /// wrapspace: hanging indent of continuation rows, in columns.
    pub wrapspace: usize,
}

struct Metrics {
    font_size: Pixels,
    row: f32,
    ascent: f32,
    descent: f32,
    ch: f32,
}

/// A line shaped and wrapped at the current width.
struct LaidLine {
    styled: Arc<StyledLine>,
    layout: Arc<LineLayout>,
    /// The text runs it was shaped with (for reshaping a right-to-left row on its own).
    text_runs: Vec<TextRun>,
    /// x of every byte boundary, in logical order, with cells one column wide and tabs at
    /// eight-column stops.
    x: Vec<f32>,
    /// For a glyph, how far it sits from the start of its cluster (a combining mark's
    /// offset over its base letter).
    cluster_origin: Vec<f32>,
    rows: Vec<Range<usize>>,
    /// Per byte: inside a run drawn as cells.
    is_cell: Vec<bool>,
    /// Holds right-to-left text: such rows are reshaped for display.
    rtl: bool,
}

struct VisibleRow {
    line: usize,
    row: usize,
    top: f32,
}

pub struct Prepared {
    snapshot: OutputSnapshot,
    metrics: Metrics,
    lines: Vec<LaidLine>,
    rows: Vec<VisibleRow>,
    content: Bounds<Pixels>,
}

pub fn output_view(snapshot: OutputSnapshot) -> impl IntoElement {
    canvas(move |bounds, window, cx| prepare(snapshot, bounds, window, cx), paint).size_full()
}

/// Whether `font` has a glyph for `c` (cached: asked once per font and character).
fn covers(window: &Window, font: &Font, c: char) -> bool {
    use std::cell::RefCell;
    use std::collections::HashMap;
    thread_local! {
        static COVERAGE: RefCell<HashMap<(SharedString, char), bool>> = RefCell::new(HashMap::new());
    }
    COVERAGE.with(|cache| {
        *cache.borrow_mut().entry((font.family.clone(), c)).or_insert_with(|| {
            let ts = window.text_system();
            ts.advance(ts.resolve_font(font), px(16.0), c).is_ok()
        })
    })
}

/// The text runs a line is shaped with: style runs split wherever the font changes,
/// choosing the font per grapheme cluster as the browser's subset fallback does. Like a
/// browser, a font is only used for a cluster it covers: when the fallback text font
/// lacks one of its characters (DejaVu Sans Mono has no precomposed "ế", so a shaper
/// would split it into "ê" and an accent the font can't attach), the cluster goes to
/// JetBrains Mono instead, which covers far more of Latin than the web's subset did.
fn text_runs(line: &StyledLine, fonts: &Fonts, window: &Window) -> Vec<TextRun> {
    let mut runs: Vec<TextRun> = Vec::new();
    let mut at = 0;
    for run in &line.runs {
        let seg = &line.text[at..at + run.len];
        for g in seg.graphemes(true) {
            let mut class = class_of(g);
            if class == FontClass::Text {
                let text_font = fonts.font(FontClass::Text, false, false);
                let primary = fonts.font(FontClass::Primary, false, false);
                let usable = |c: &char| is_invisible(*c) || covers(window, &text_font, *c);
                if !g.chars().all(|c| usable(&c)) && g.chars().all(|c| is_invisible(c) || covers(window, &primary, c)) {
                    class = FontClass::Primary;
                }
            }
            let font: Font = fonts.font(class, run.bold, run.italic);
            match runs.last_mut() {
                Some(last) if last.font == font => last.len += g.len(),
                _ => runs.push(TextRun { len: g.len(), font, color: Hsla::default(), ..Default::default() }),
            }
        }
        at += run.len;
    }
    runs
}

/// Characters a browser never draws and gives no width: zero-width spaces and joiners,
/// direction marks and isolates, variation selectors, the BOM.
fn is_invisible(c: char) -> bool {
    matches!(c as u32, 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x2064 | 0x2066..=0x206F
        | 0xFE00..=0xFE0F | 0xFEFF)
}

fn is_rtl(c: char) -> bool {
    use unicode_bidi::{BidiClass, bidi_class};
    matches!(bidi_class(c), BidiClass::R | BidiClass::AL)
}

/// CSS `tab-size: 8`.
const TAB_COLUMNS: f32 = 8.0;

fn lay_out(styled: Arc<StyledLine>, snapshot: &OutputSnapshot, m: &Metrics, width: f32, window: &Window) -> LaidLine {
    let text = &styled.text;
    let n = text.len();
    let text_runs = text_runs(&styled, &snapshot.fonts, window);
    let layout = window.text_system().layout_line(text, m.font_size, &text_runs, None);

    // Each cluster's advance, whatever the direction: glyphs in visual order, each one's
    // advance the gap to the next, summed per cluster. A cluster's own origin is its
    // leftmost glyph, so its other glyphs (combining marks) keep their offsets from it.
    let mut glyphs: Vec<(f32, usize)> = layout
        .runs
        .iter()
        .flat_map(|r| r.glyphs.iter().map(|g| (f32::from(g.position.x), g.index)))
        .filter(|(_, i)| *i < n)
        .collect();
    glyphs.sort_by(|a, b| a.0.total_cmp(&b.0));
    if let Ok(probe) = std::env::var("CLAY_NATIVE_GLYPH_PROBE") {
        if !probe.is_empty() && text.contains(probe.as_str()) {
            for r in &layout.runs {
                for g in &r.glyphs {
                    crate::debug_log(true, &format!("GLYPH font={:?} id={:?} index={} x={:.2} y={:.2} emoji={} char={:?}",
                        r.font_id, g.id, g.index, f32::from(g.position.x), f32::from(g.position.y), g.is_emoji,
                        text[g.index.min(n)..].chars().next()));
                }
            }
        }
    }
    let mut advance = vec![0.0f32; n];
    let mut cluster_origin = vec![f32::INFINITY; n];
    for (k, &(gx, i)) in glyphs.iter().enumerate() {
        let next = glyphs.get(k + 1).map(|g| g.0).unwrap_or(f32::from(layout.width));
        advance[i] += (next - gx).max(0.0);
        cluster_origin[i] = cluster_origin[i].min(gx);
    }

    let mut is_cell = vec![false; n];
    let mut at = 0;
    for run in &styled.runs {
        if run.cell.is_some() {
            is_cell[at..at + run.len].iter_mut().for_each(|c| *c = true);
        }
        at += run.len;
    }

    // Positions in logical order: cells exactly one column (`width: Nch`), tabs to the
    // next eight-column stop, invisible characters nothing.
    let tab = TAB_COLUMNS * m.ch;
    let mut x = vec![0.0f32; n + 1];
    let mut cur = 0.0f32;
    let mut rtl = false;
    for (i, c) in text.char_indices() {
        let j = i + c.len_utf8();
        rtl |= is_rtl(c);
        let adv = if is_cell[i] {
            m.ch
        } else if c == '\t' {
            tab - (cur % tab)
        } else if is_invisible(c) {
            0.0
        } else {
            advance[i]
        };
        for slot in &mut x[i..j] {
            *slot = cur;
        }
        cur += adv;
    }
    x[n] = cur;

    let indent = snapshot.wrapspace as f32 * m.ch;
    let rows = wrap_rows(text, &styled.breaks, width, indent, &|i| x[i]);
    LaidLine { styled, layout, text_runs, x, cluster_origin, rows, is_cell, rtl }
}

/// The text runs for part of a line (a right-to-left row reshaped on its own).
fn sub_runs(runs: &[TextRun], range: &Range<usize>) -> Vec<TextRun> {
    let mut out = Vec::new();
    let mut at = 0;
    for r in runs {
        let (s, e) = (at.max(range.start), (at + r.len).min(range.end));
        at += r.len;
        if s < e {
            let mut sub = r.clone();
            sub.len = e - s;
            out.push(sub);
        }
    }
    out
}

fn prepare(snapshot: OutputSnapshot, bounds: Bounds<Pixels>, window: &mut Window, _cx: &mut App) -> Prepared {
    let font_size = px(snapshot.font_size);
    let ts = window.text_system();
    let primary = ts.resolve_font(&snapshot.fonts.font(FontClass::Primary, false, false));
    let metrics = Metrics {
        font_size,
        row: snapshot.font_size,
        ascent: f32::from(ts.ascent(primary, font_size)),
        // GPUI reports the descent as a (negative) offset below the baseline.
        descent: f32::from(ts.descent(primary, font_size)).abs(),
        ch: ts.ch_advance(primary, font_size).map(f32::from).unwrap_or(snapshot.font_size * 0.6),
    };

    let content = Bounds::new(
        point(bounds.origin.x + px(PADDING), bounds.origin.y + px(PADDING)),
        size(bounds.size.width - px(2.0 * PADDING), bounds.size.height - px(2.0 * PADDING)),
    );
    let width = f32::from(content.size.width).max(metrics.ch);
    let visible = (f32::from(content.size.height) / metrics.row).ceil() as usize + 1;
    let wanted = visible + snapshot.scroll_rows;

    // Lay out from the newest line upward until the screen (plus the scroll) is full.
    let mut lines: Vec<LaidLine> = Vec::new();
    let mut total_rows = 0usize;
    for styled in snapshot.lines.iter().rev() {
        let laid = lay_out(styled.clone(), &snapshot, &metrics, width, window);
        total_rows += laid.rows.len();
        lines.push(laid);
        if total_rows >= wanted {
            break;
        }
    }
    lines.reverse();

    // Rows top to bottom. Following the bottom, the newest row sits on the bottom edge;
    // with less than a screenful, rows start at the top as in the browser.
    let all: Vec<(usize, usize)> =
        lines.iter().enumerate().flat_map(|(li, l)| (0..l.rows.len()).map(move |ri| (li, ri))).collect();
    let fits = (f32::from(content.size.height) / metrics.row).floor() as usize;
    let mut rows = Vec::new();
    if all.len() <= fits {
        for (k, (li, ri)) in all.iter().enumerate() {
            rows.push(VisibleRow { line: *li, row: *ri, top: f32::from(content.origin.y) + k as f32 * metrics.row });
        }
    } else {
        let bottom = f32::from(content.origin.y + content.size.height);
        let end = all.len().saturating_sub(snapshot.scroll_rows.min(all.len()));
        for (k, (li, ri)) in all[..end].iter().rev().enumerate() {
            let top = bottom - (k + 1) as f32 * metrics.row;
            if top + metrics.row < f32::from(content.origin.y) {
                break;
            }
            rows.push(VisibleRow { line: *li, row: *ri, top });
        }
        rows.reverse();
    }

    Prepared { snapshot, metrics, lines, rows, content }
}

fn paint(bounds: Bounds<Pixels>, p: Prepared, window: &mut Window, _cx: &mut App) {
    window.paint_quad(fill(bounds, hsla(p.snapshot.background)));
    let m = &p.metrics;
    let indent = p.snapshot.wrapspace as f32 * m.ch;
    // Where the font's own content area starts within a row (CSS half-leading).
    let half_leading = (m.row - (m.ascent + m.descent)) / 2.0;

    window.with_content_mask(Some(ContentMask { bounds }), |window| {
        for vr in &p.rows {
            let line = &p.lines[vr.line];
            let range = line.rows[vr.row].clone();
            let left = f32::from(p.content.origin.x) + if vr.row > 0 { indent } else { 0.0 };
            let x0 = line.x[range.start];
            let baseline = vr.top + half_leading + m.ascent;

            // Every coloured run's background, over the font's full height (CSS paints
            // an inline box's background over its content area, taller than the row).
            // Cells repaint their own exact boxes on top later.
            let mut at = 0;
            for run in &line.styled.runs {
                let (s, e) = (at, at + run.len);
                at = e;
                let (s, e) = (s.max(range.start), e.min(range.end));
                if s >= e {
                    continue;
                }
                if let Some(bg) = run.bg {
                    let bx = left + line.x[s] - x0;
                    let bw = line.x[e] - line.x[s];
                    window.paint_quad(fill(
                        Bounds::new(point(px(bx), px(vr.top + half_leading)), size(px(bw), px(m.ascent + m.descent))),
                        hsla(bg),
                    ));
                }
            }

            // Glyphs.
            let mut style_ends: Vec<(usize, Rgb)> = Vec::new();
            let mut at = 0;
            for run in &line.styled.runs {
                at += run.len;
                style_ends.push((at, run.fg));
            }
            let fg_at = |i: usize| style_ends.iter().find(|(end, _)| i < *end).map(|(_, c)| *c).unwrap_or([255, 255, 255]);
            let text = &line.styled.text;
            let skip = |i: usize| line.is_cell[i] || text[i..].chars().next().is_some_and(|c| c == '\t' || is_invisible(c));
            let row_text = &text[range.clone()];
            if line.rtl && row_text.chars().any(is_rtl) {
                // Right-to-left text: order the row with the Unicode bidi algorithm, then
                // shape each visual run on its own (which also joins Arabic letters) and
                // lay the runs out left to right.
                let bidi = unicode_bidi::BidiInfo::new(row_text, None);
                let mut pen = left;
                for para in &bidi.paragraphs {
                    let (levels, visual) = bidi.visual_runs(para, para.range.clone());
                    for vr in visual {
                        let rtl_run = levels.get(vr.start).is_some_and(|l| l.is_rtl());
                        let span = range.start + vr.start..range.start + vr.end;
                        let runs = sub_runs(&line.text_runs, &span);
                        let run_layout = window.text_system().layout_line(&text[span.clone()], m.font_size, &runs, None);
                        let width = f32::from(run_layout.width);
                        // GPUI's DirectWrite path lays every glyph run out left to right in
                        // logical order (it ignores the run's bidi level), so a right-to-left
                        // run comes back reversed: mirror it. The other platforms' shapers
                        // already return visual positions.
                        let mirror = cfg!(target_os = "windows") && rtl_run;
                        let xs: Vec<f32> = run_layout.runs.iter().flat_map(|r| r.glyphs.iter().map(|g| f32::from(g.position.x))).collect();
                        let mut k = 0usize;
                        for run in &run_layout.runs {
                            for g in &run.glyphs {
                                let gx = xs[k];
                                let next = xs.iter().skip(k + 1).copied().find(|&x| x > gx).unwrap_or(width);
                                k += 1;
                                let i = span.start + g.index;
                                if i >= span.end || skip(i) {
                                    continue;
                                }
                                let x = if mirror { width - next } else { gx };
                                let origin = point(px(pen + x), px(baseline) + g.position.y);
                                let _ = if g.is_emoji {
                                    window.paint_emoji(origin, run.font_id, g.id, m.font_size)
                                } else {
                                    window.paint_glyph(origin, run.font_id, g.id, m.font_size, hsla(fg_at(i)))
                                };
                            }
                        }
                        pen += f32::from(run_layout.width);
                    }
                }
            } else {
                for run in &line.layout.runs {
                    for g in &run.glyphs {
                        let i = g.index;
                        if i < range.start || i >= range.end || skip(i) {
                            continue;
                        }
                        let within = f32::from(g.position.x) - line.cluster_origin[i];
                        let origin = point(px(left + line.x[i] - x0 + within), px(baseline) + g.position.y);
                        let _ = if g.is_emoji {
                            window.paint_emoji(origin, run.font_id, g.id, m.font_size)
                        } else {
                            window.paint_glyph(origin, run.font_id, g.id, m.font_size, hsla(fg_at(i)))
                        };
                    }
                }
            }

            // Underlines.
            let mut at = 0;
            for run in &line.styled.runs {
                let (s, e) = (at, at + run.len);
                at = e;
                let (s, e) = (s.max(range.start), e.min(range.end));
                if s >= e || !run.underline || run.cell.is_some() {
                    continue;
                }
                let thickness = (m.row / 14.0).max(1.0);
                window.paint_underline(
                    point(px(left + line.x[s] - x0), px(baseline + m.descent * 0.35)),
                    px(line.x[e] - line.x[s]),
                    &UnderlineStyle { thickness: px(thickness), color: Some(hsla(run.fg)), wavy: false },
                );
            }
        }

        // Cells last, over every row's text and backgrounds.
        for vr in &p.rows {
            let line = &p.lines[vr.line];
            let range = line.rows[vr.row].clone();
            let left = f32::from(p.content.origin.x) + if vr.row > 0 { indent } else { 0.0 };
            let x0 = line.x[range.start];
            let mut at = 0;
            for run in &line.styled.runs {
                let (s, e) = (at, at + run.len);
                at = e;
                let Some(cell) = run.cell else { continue };
                let (s, e) = (s.max(range.start), e.min(range.end));
                if s >= e {
                    continue;
                }
                for (off, _) in line.styled.text[s..e].char_indices() {
                    let i = s + off;
                    let cell_box = Bounds::new(point(px(left + line.x[i] - x0), px(vr.top)), size(px(m.ch), px(m.row)));
                    paint_cell(window, cell_box, cell, run.fg, run.bg);
                }
            }
        }
    });
}

/// A block element's shape, from app.js `BLOCK_ELEMENT_LAYERS`, over the cell's background.
fn paint_cell(window: &mut Window, cell: Bounds<Pixels>, kind: Cell, fg: Rgb, bg: Option<Rgb>) {
    // Snap to device pixels so neighbouring cells meet without seams.
    let scale = window.scale_factor();
    let snap = |v: f32| (v * scale).round() / scale;
    let (x, y) = (f32::from(cell.origin.x), f32::from(cell.origin.y));
    let (w, h) = (f32::from(cell.size.width), f32::from(cell.size.height));
    let rect = |window: &mut Window, fx0: f32, fy0: f32, fx1: f32, fy1: f32, colour: Hsla| {
        let (x0, y0, x1, y1) = (snap(x + w * fx0), snap(y + h * fy0), snap(x + w * fx1), snap(y + h * fy1));
        if x1 > x0 && y1 > y0 {
            window.paint_quad(fill(Bounds::new(point(px(x0), px(y0)), size(px(x1 - x0), px(y1 - y0))), colour));
        }
    };
    if let Some(bg) = bg {
        rect(window, 0.0, 0.0, 1.0, 1.0, hsla(bg));
    }
    let c = hsla(fg);
    let Cell::Block(ch) = kind else { return };
    let eighth = |n: u32| n as f32 / 8.0;
    match ch {
        '▀' => rect(window, 0.0, 0.0, 1.0, 0.5, c),
        '\u{2581}'..='\u{2587}' => {
            let n = ch as u32 - 0x2580;
            rect(window, 0.0, 1.0 - eighth(n), 1.0, 1.0, c);
        }
        '█' => rect(window, 0.0, 0.0, 1.0, 1.0, c),
        '\u{2589}'..='\u{258F}' => {
            let n = 7 - (ch as u32 - 0x2589);
            rect(window, 0.0, 0.0, eighth(n), 1.0, c);
        }
        '▐' => rect(window, 0.5, 0.0, 1.0, 1.0, c),
        '░' => rect(window, 0.0, 0.0, 1.0, 1.0, hsla_alpha(fg, 0.25)),
        '▒' => rect(window, 0.0, 0.0, 1.0, 1.0, hsla_alpha(fg, 0.5)),
        '▓' => rect(window, 0.0, 0.0, 1.0, 1.0, hsla_alpha(fg, 0.75)),
        '▔' => rect(window, 0.0, 0.0, 1.0, eighth(1), c),
        '▕' => rect(window, 1.0 - eighth(1), 0.0, 1.0, 1.0, c),
        _ => {
            // Quadrants: upper-left, upper-right, lower-left, lower-right.
            let (ul, ur, ll, lr) = match ch {
                '▖' => (false, false, true, false),
                '▗' => (false, false, false, true),
                '▘' => (true, false, false, false),
                '▙' => (true, false, true, true),
                '▚' => (true, false, false, true),
                '▛' => (true, true, true, false),
                '▜' => (true, true, false, true),
                '▝' => (false, true, false, false),
                '▞' => (false, true, true, false),
                '▟' => (false, true, true, true),
                _ => (false, false, false, false),
            };
            if ul { rect(window, 0.0, 0.0, 0.5, 0.5, c); }
            if ur { rect(window, 0.5, 0.0, 1.0, 0.5, c); }
            if ll { rect(window, 0.0, 0.5, 0.5, 1.0, c); }
            if lr { rect(window, 0.5, 0.5, 1.0, 1.0, c); }
        }
    }
}
