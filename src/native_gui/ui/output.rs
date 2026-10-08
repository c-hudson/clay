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
//! - A Discord emoji is an image 1.2em tall centred on the text's middle, as the web's
//!   `vertical-align: middle` puts it. Where the browser makes that row taller, here rows
//!   stay one em, so the image reaches a little into the rows around it; it paints last.
//! - Underlines skip descenders (`text-decoration-skip-ink`), see `ink`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use gpui::{
    App, Bounds, ContentMask, Corners, Font, FontId, Hsla, IntoElement, LineLayout, Pixels, Rgba, SharedString,
    Styled, TextRun, UnderlineStyle, Window, canvas, fill, point, px, size,
};
use unicode_segmentation::UnicodeSegmentation;

use super::emoji::{self, Look};
use super::fonts::{FontClass, Fonts, bundled_index, class_of};
use super::ink;
use crate::native_gui::core::style::{Cell, DiscordEmoji, Rgb, Run, StyledLine};
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
    /// The lines shown, oldest first, each with its `Line::id`.
    pub lines: Vec<(u64, Arc<StyledLine>)>,
    pub background: Rgb,
    pub font_size: f32,
    pub fonts: Fonts,
    /// wrapspace: hanging indent of continuation rows, in columns.
    pub wrapspace: usize,
    /// Changes whenever the same line would lay out differently (styling, show_tags).
    pub style_epoch: u64,
    pub scroll: ScrollHandle,
}

/// Where the view is. The window moves it (keys, wheel, more-mode); the layout resolves
/// the move against the lines' real row counts and reports back what it found.
///
/// Following the bottom, new output scrolls into view. Otherwise the view is anchored to
/// a line, so neither output arriving below nor history arriving above moves what the
/// user is reading. Only the user leaves the bottom; reaching it again follows again
/// (app.js `followBottom`).
pub struct Scroll {
    pub follow: bool,
    /// When not following: the top row shown, as (line id, row within the line).
    top: Option<(u64, usize)>,
    /// Rows to move at the next layout; negative is up.
    pending: isize,
    /// After the last layout: rows below the view (the History count).
    pub rows_below: usize,
    /// After the last layout: whole rows the view holds.
    pub view_rows: usize,
    /// Rows per line at the current width, font and style, so counting rows below the
    /// view costs a layout only once per line.
    rows_cache: (u32, u32, u64),
    rows: HashMap<u64, usize>,
}

pub type ScrollHandle = Rc<RefCell<Scroll>>;

impl Default for Scroll {
    fn default() -> Self {
        Scroll { follow: true, top: None, pending: 0, rows_below: 0, view_rows: 24, rows_cache: (0, 0, 0), rows: HashMap::new() }
    }
}

impl Scroll {
    /// Move by `rows` (negative is up).
    pub fn by(&mut self, rows: isize) {
        self.pending += rows;
    }

    /// Jump to the bottom and follow it.
    pub fn to_bottom(&mut self) {
        self.follow = true;
        self.top = None;
        self.pending = 0;
    }

    pub fn at_bottom(&self) -> bool {
        self.follow || self.rows_below == 0
    }
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
    /// Per byte: drawn here rather than as a glyph (a cell, or a Discord emoji).
    no_glyph: Vec<bool>,
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
    /// The bundled faces' font ids, by [`bundled_index`]: glyphs whose outlines `ink` can
    /// read.
    bundled: [FontId; 4],
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

    // Cells are exactly one column each (`width: Nch`); an emoji is as wide as its image.
    let mut no_glyph = vec![false; n];
    let mut fixed: Vec<Option<f32>> = vec![None; n];
    let mut at = 0;
    for run in &styled.runs {
        let span = at..at + run.len;
        at += run.len;
        if run.cell.is_some() {
            no_glyph[span.clone()].fill(true);
            for (off, _) in text[span.clone()].char_indices() {
                fixed[span.start + off] = Some(m.ch);
            }
        }
        if let Some(e) = &run.emoji {
            no_glyph[span.clone()].fill(true);
            fixed[span.start] = Some(emoji_width(e, m));
        }
    }

    // Positions in logical order: cells and emoji as above, tabs to the next
    // eight-column stop, invisible characters nothing.
    let tab = TAB_COLUMNS * m.ch;
    let mut x = vec![0.0f32; n + 1];
    let mut cur = 0.0f32;
    let mut rtl = false;
    for (i, c) in text.char_indices() {
        let j = i + c.len_utf8();
        rtl |= is_rtl(c);
        let adv = if let Some(w) = fixed[i] {
            w
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
    LaidLine { styled, layout, text_runs, x, cluster_origin, rows, no_glyph, rtl }
}

/// How wide a Discord emoji is: its image at the web's height, a square until the image
/// arrives, or its `:name:` if it never will.
fn emoji_width(e: &DiscordEmoji, m: &Metrics) -> f32 {
    let h = emoji::HEIGHT_EM * f32::from(m.font_size);
    match emoji::lookup(e) {
        Look::Ready(l) => h * l.aspect,
        Look::Loading => h,
        Look::Failed => (e.name.chars().count() + 2) as f32 * m.ch,
    }
}

/// Where the view lands: `top` is the first row shown (line, row within it), or `None`
/// when the view sits on the bottom; `below` is how many rows are under the view.
#[derive(Debug, PartialEq, Eq)]
struct ViewPlan {
    top: Option<(usize, usize)>,
    below: usize,
}

/// Resolve a scroll against the lines' row counts (`rows(i)`, at least 1 each): start
/// from the anchor (or from the bottom when following), move `pending` rows, and see
/// whether what follows fills the view — if it doesn't, the view is at the bottom.
fn plan_view(
    n: usize,
    rows: &mut dyn FnMut(usize) -> usize,
    follow: bool,
    anchored: Option<(usize, usize)>,
    pending: isize,
    view_rows: usize,
) -> ViewPlan {
    if n == 0 || (follow && pending >= 0) {
        return ViewPlan { top: None, below: 0 };
    }
    let mut pos = match anchored {
        Some((i, r)) => (i, r.min(rows(i) - 1)),
        None => {
            // A screenful up from the end.
            let (mut i, mut need) = (n - 1, view_rows);
            loop {
                let r = rows(i);
                if r >= need || i == 0 {
                    break (i, r.saturating_sub(need));
                }
                need -= r;
                i -= 1;
            }
        }
    };
    let mut delta = pending;
    while delta < 0 {
        if pos.1 > 0 {
            pos.1 -= 1;
        } else if pos.0 > 0 {
            pos.0 -= 1;
            pos.1 = rows(pos.0) - 1;
        } else {
            break;
        }
        delta += 1;
    }
    while delta > 0 {
        if pos.1 + 1 < rows(pos.0) {
            pos.1 += 1;
        } else if pos.0 + 1 < n {
            pos = (pos.0 + 1, 0);
        } else {
            break;
        }
        delta -= 1;
    }
    let mut have = rows(pos.0) - pos.1;
    let mut i = pos.0;
    while have < view_rows && i + 1 < n {
        i += 1;
        have += rows(i);
    }
    if have <= view_rows && i + 1 >= n {
        return ViewPlan { top: None, below: 0 };
    }
    let mut below = have - view_rows;
    for j in i + 1..n {
        below += rows(j);
    }
    ViewPlan { top: Some(pos), below }
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
    let height = f32::from(content.size.height);
    let view_rows = ((height / metrics.row).floor() as usize).max(1);
    let n = snapshot.lines.len();
    let scroll = snapshot.scroll.clone();
    let mut st = scroll.borrow_mut();
    let key = (width.to_bits(), snapshot.font_size.to_bits(), snapshot.style_epoch);
    if st.rows_cache != key {
        st.rows_cache = key;
        st.rows.clear();
    }

    // Lines are laid out only as far as the view (and the scroll) needs.
    let mut laid: HashMap<usize, LaidLine> = HashMap::new();
    let rows_of = |i: usize, st: &mut Scroll, laid: &mut HashMap<usize, LaidLine>| -> usize {
        let id = snapshot.lines[i].0;
        if let Some(&r) = st.rows.get(&id) {
            return r;
        }
        let l = lay_out(snapshot.lines[i].1.clone(), &snapshot, &metrics, width, window);
        let r = l.rows.len().max(1);
        st.rows.insert(id, r);
        laid.insert(i, l);
        r
    };

    // Where the view is, worked out against the lines' row counts.
    let anchored = if st.follow { None } else { st.top.and_then(|(id, r)| snapshot.lines.iter().position(|l| l.0 == id).map(|i| (i, r))) };
    let (follow, pending) = (st.follow, st.pending);
    let plan = {
        let mut rows = |i: usize| rows_of(i, &mut st, &mut laid);
        plan_view(n, &mut rows, follow, anchored, pending, view_rows)
    };
    let (top, below) = (plan.top, plan.below);
    st.pending = 0;
    match top {
        Some((ti, tr)) => {
            st.follow = false;
            st.top = Some((snapshot.lines[ti].0, tr));
        }
        None => {
            st.follow = true;
            st.top = None;
        }
    }
    st.rows_below = below;
    st.view_rows = view_rows;
    drop(st);

    // The visible rows, top to bottom.
    let mut order: Vec<usize> = Vec::new();
    let mut rows = Vec::new();
    let place = |i: usize, laid: &mut HashMap<usize, LaidLine>, order: &mut Vec<usize>| -> usize {
        if !laid.contains_key(&i) {
            laid.insert(i, lay_out(snapshot.lines[i].1.clone(), &snapshot, &metrics, width, window));
        }
        match order.iter().position(|&o| o == i) {
            Some(p) => p,
            None => {
                order.push(i);
                order.len() - 1
            }
        }
    };
    match top {
        Some((ti, tr)) => {
            let (mut i, mut r, mut k) = (ti, tr, 0usize);
            while k <= view_rows && i < n {
                let li = place(i, &mut laid, &mut order);
                let count = laid[&i].rows.len().max(1);
                while r < count && k <= view_rows {
                    rows.push(VisibleRow { line: li, row: r, top: f32::from(content.origin.y) + k as f32 * metrics.row });
                    r += 1;
                    k += 1;
                }
                i += 1;
                r = 0;
            }
        }
        None if n > 0 => {
            // Bottom-anchored: the newest row on the bottom edge; with less than a
            // screenful, rows start at the top as in the browser.
            let mut stack: Vec<(usize, usize)> = Vec::new();
            let mut i = n;
            while i > 0 && stack.len() <= view_rows {
                i -= 1;
                let count = rows_of(i, &mut scroll.borrow_mut(), &mut laid);
                for r in (0..count).rev() {
                    stack.push((i, r));
                }
            }
            stack.reverse();
            let fits = stack.len() <= view_rows;
            let skip = stack.len().saturating_sub(view_rows + 1);
            let bottom = f32::from(content.origin.y + content.size.height);
            let shown: Vec<(usize, usize)> = stack[skip..].to_vec();
            let count = shown.len();
            for (k, (i, r)) in shown.into_iter().enumerate() {
                let li = place(i, &mut laid, &mut order);
                let top = if fits {
                    f32::from(content.origin.y) + k as f32 * metrics.row
                } else {
                    bottom - (count - k) as f32 * metrics.row
                };
                rows.push(VisibleRow { line: li, row: r, top });
            }
        }
        None => {}
    }
    let lines: Vec<LaidLine> = order.iter().map(|i| laid.remove(i).expect("laid out")).collect();

    let ts = window.text_system();
    let bundled = [(false, false), (true, false), (false, true), (true, true)]
        .map(|(b, i)| ts.resolve_font(&snapshot.fonts.font(FontClass::Primary, b, i)));
    debug_assert!((0..4).all(|k| bundled_index(k & 1 == 1, k & 2 == 2) == k));
    Prepared { snapshot, metrics, lines, rows, content, bundled }
}

fn paint(bounds: Bounds<Pixels>, p: Prepared, window: &mut Window, _cx: &mut App) {
    window.paint_quad(fill(bounds, hsla(p.snapshot.background)));
    let m = &p.metrics;
    let indent = p.snapshot.wrapspace as f32 * m.ch;
    // Where the font's own content area starts within a row (CSS half-leading).
    let half_leading = (m.row - (m.ascent + m.descent)) / 2.0;
    let ul = Underline::new(m);

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

            // Glyphs, noting where they cross an underline.
            let mut style_ends: Vec<(usize, Rgb, bool)> = Vec::new();
            let mut at = 0;
            for run in &line.styled.runs {
                at += run.len;
                style_ends.push((at, run.fg, underlined(run)));
            }
            let style_at = |i: usize| {
                style_ends.iter().find(|(end, ..)| i < *end).map(|&(_, c, u)| (c, u)).unwrap_or(([255, 255, 255], false))
            };
            let fg_at = |i: usize| style_at(i).0;
            let text = &line.styled.text;
            let skip = |i: usize| line.no_glyph[i] || text[i..].chars().next().is_some_and(|c| c == '\t' || is_invisible(c));
            let mut gaps: Vec<(f32, f32)> = Vec::new();
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
                        if !g.is_emoji && style_at(i).1 {
                            if let Some(face) = p.bundled.iter().position(|f| *f == run.font_id) {
                                underline_gaps(&mut gaps, &ul, face, g.id.0, f32::from(origin.x), f32::from(g.position.y));
                            }
                        }
                    }
                }
            }

            // Underlines, with the gaps the glyphs above made.
            let mut at = 0;
            for run in &line.styled.runs {
                let (s, e) = (at, at + run.len);
                at = e;
                let (s, e) = (s.max(range.start), e.min(range.end));
                if s >= e || !underlined(run) {
                    continue;
                }
                let (sx, ex) = (left + line.x[s] - x0, left + line.x[e] - x0);
                for (a, b) in ink::cut(sx, ex, &gaps) {
                    if b - a >= 0.5 {
                        window.paint_underline(
                            point(px(a), px(baseline + ul.offset)),
                            px(b - a),
                            &UnderlineStyle { thickness: px(ul.thickness), color: Some(hsla(run.fg)), wavy: false },
                        );
                    }
                }
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

        // Discord emoji last: an image reaches past its row.
        let now = Instant::now();
        let mut next_frame: Option<Instant> = None;
        let height = emoji::HEIGHT_EM * f32::from(m.font_size);
        let middle = ink::x_height_em() * f32::from(m.font_size) / 2.0;
        for vr in &p.rows {
            let line = &p.lines[vr.line];
            let range = line.rows[vr.row].clone();
            let left = f32::from(p.content.origin.x) + if vr.row > 0 { indent } else { 0.0 };
            let x0 = line.x[range.start];
            let baseline = vr.top + half_leading + m.ascent;
            let mut at = 0;
            for run in &line.styled.runs {
                let s = at;
                at += run.len;
                let Some(e) = &run.emoji else { continue };
                if !range.contains(&s) {
                    continue;
                }
                let x = left + line.x[s] - x0;
                match emoji::lookup(e) {
                    Look::Ready(l) => {
                        let b = Bounds::new(
                            point(px(x), px(baseline - middle - height / 2.0)),
                            size(px(height * l.aspect), px(height)),
                        );
                        let (frame, next) = l.frame_at(now);
                        let _ = window.paint_image(b, b, Corners::default(), l.image.clone(), frame, false);
                        if let Some(t) = next {
                            next_frame = Some(next_frame.map_or(t, |n| n.min(t)));
                        }
                    }
                    Look::Failed => paint_emoji_name(window, &p, e, run, x, baseline),
                    Look::Loading => {}
                }
            }
        }
        if let Some(t) = next_frame {
            emoji::wake_at(t);
        }
    });
}

/// Whether a run's text is underlined (cells and emoji never are).
fn underlined(run: &Run) -> bool {
    run.underline && run.cell.is_none() && run.emoji.is_none()
}

/// Where the underline sits: below the baseline, in the run's colour.
struct Underline {
    /// From the baseline down to the line's top edge.
    offset: f32,
    thickness: f32,
    /// How far a gap reaches past the ink on each side: the line's thickness, as Blink
    /// does (capped at 13px there too).
    clearance: f32,
    font_size: f32,
}

impl Underline {
    fn new(m: &Metrics) -> Self {
        let thickness = (m.row / 14.0).max(1.0);
        Underline {
            offset: m.descent * 0.35,
            thickness,
            clearance: thickness.round().clamp(1.0, 13.0),
            font_size: f32::from(m.font_size),
        }
    }
}

/// Add the gaps a bundled-font glyph at `origin_x` (drawn `dy` below the baseline) cuts
/// into the underline.
fn underline_gaps(gaps: &mut Vec<(f32, f32)>, ul: &Underline, face: usize, glyph: u32, origin_x: f32, dy: f32) {
    let Some(upem) = ink::units_per_em(face) else { return };
    let k = upem / ul.font_size;
    // The band in the glyph's own units, y up from its baseline.
    let top = ul.offset - dy;
    let (lo, hi) = (-(top + ul.thickness) * k, -top * k);
    for &(a, b) in ink::spans(face, glyph, lo, hi).iter() {
        gaps.push((origin_x + a / k - ul.clearance, origin_x + b / k + ul.clearance));
    }
}

/// An emoji Discord couldn't supply, shown as its name (the web shows a broken image).
fn paint_emoji_name(window: &mut Window, p: &Prepared, e: &DiscordEmoji, run: &Run, x: f32, baseline: f32) {
    let m = &p.metrics;
    let name = format!(":{}:", e.name);
    let font = p.snapshot.fonts.font(FontClass::Primary, run.bold, run.italic);
    let runs = [TextRun { len: name.len(), font, color: Hsla::default(), ..Default::default() }];
    let layout = window.text_system().layout_line(&name, m.font_size, &runs, None);
    for r in &layout.runs {
        for g in &r.glyphs {
            let origin = point(px(x) + g.position.x, px(baseline) + g.position.y);
            let _ = window.paint_glyph(origin, r.font_id, g.id, m.font_size, hsla(run.fg));
        }
    }
}

/// A box-drawing character as exact lines (`boxdraw`), in device pixels so they land on
/// whole pixels and meet the next cell's.
fn paint_box(window: &mut Window, cell: Bounds<Pixels>, ch: char, colour: Hsla) {
    use crate::native_gui::core::boxdraw::{CellPx, Seg, Thickness, shape};
    let s = window.scale_factor();
    let dev = |v: Pixels| (f32::from(v) * s).round();
    let px_box = CellPx {
        x0: dev(cell.origin.x),
        x1: dev(cell.origin.x + cell.size.width),
        y0: dev(cell.origin.y),
        y1: dev(cell.origin.y + cell.size.height),
    };
    let Some(sh) = shape(ch, px_box, Thickness::for_row(px_box.y1 - px_box.y0)) else { return };
    let lp = |v: f32| px(v / s);
    for [x0, y0, x1, y1] in sh.rects {
        window.paint_quad(fill(Bounds::new(point(lp(x0), lp(y0)), size(lp(x1 - x0), lp(y1 - y0))), colour));
    }
    for (width, segs) in sh.strokes {
        let mut path = gpui::PathBuilder::stroke(lp(width));
        for seg in segs {
            match seg {
                Seg::Move(x, y) => path.move_to(point(lp(x), lp(y))),
                Seg::Line(x, y) => path.line_to(point(lp(x), lp(y))),
                Seg::Cubic([ax, ay, bx, by, x, y]) => {
                    path.cubic_bezier_to(point(lp(x), lp(y)), point(lp(ax), lp(ay)), point(lp(bx), lp(by)))
                }
            }
        }
        if let Ok(path) = path.build() {
            window.paint_path(path, colour);
        }
    }
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
    if let Cell::Box(ch) = kind {
        paint_box(window, cell, ch, c);
        return;
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Ten lines; line 5 wraps to three rows.
    fn rows(i: usize) -> usize {
        if i == 5 { 3 } else { 1 }
    }

    fn plan(follow: bool, anchored: Option<(usize, usize)>, pending: isize, view: usize) -> ViewPlan {
        plan_view(10, &mut rows, follow, anchored, pending, view)
    }

    #[test]
    fn following_stays_on_the_bottom() {
        assert_eq!(plan(true, None, 0, 4), ViewPlan { top: None, below: 0 });
        assert_eq!(plan(true, None, 3, 4), ViewPlan { top: None, below: 0 }, "down at the bottom stays there");
    }

    #[test]
    fn scrolling_up_from_the_bottom_anchors_by_rows() {
        // 12 rows in all (line 5 is rows 5-7), 4 shown: following, the top is row 8 =
        // line 6. Up 2 rows: the top is row 6, line 5's middle row; 2 rows under the view.
        assert_eq!(plan(true, None, -2, 4), ViewPlan { top: Some((5, 1)), below: 2 });
        // Up 4: row 4, line 4; rows 8-11 under the view.
        assert_eq!(plan(true, None, -4, 4), ViewPlan { top: Some((4, 0)), below: 4 });
    }

    #[test]
    fn an_anchor_holds_and_clamps_at_the_top() {
        assert_eq!(plan(false, Some((2, 0)), 0, 4), ViewPlan { top: Some((2, 0)), below: 6 });
        assert_eq!(plan(false, Some((1, 0)), -5, 4), ViewPlan { top: Some((0, 0)), below: 8 });
    }

    #[test]
    fn reaching_the_bottom_follows_again() {
        assert_eq!(plan(false, Some((2, 0)), 6, 4), ViewPlan { top: None, below: 0 });
        // Row 2 down 5 is row 7, line 5's last row: rows 7-10 shown, row 11 under.
        assert_eq!(plan(false, Some((2, 0)), 5, 4), ViewPlan { top: Some((5, 2)), below: 1 });
    }

    #[test]
    fn less_than_a_screenful_is_always_at_the_bottom() {
        assert_eq!(plan_view(2, &mut |_| 1, false, Some((0, 0)), -3, 10), ViewPlan { top: None, below: 0 });
    }
}
