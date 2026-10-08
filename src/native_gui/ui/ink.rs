//! Where an underline gives way to glyph ink: CSS `text-decoration-skip-ink: auto`, every
//! browser's default, which the WebView's underlines follow (the line breaks around the
//! descenders of "g", "j", "p", "q", "y").
//!
//! As a browser does it: intersect each glyph's outline with the underline's band, and
//! cut the line there, widened by the line's thickness on each side. Outlines come from
//! the bundled JetBrains Mono faces, which draw all the Latin text an underline usually
//! sits under; glyphs from other fonts keep a continuous line.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use ttf_parser::{Face, GlyphId, OutlineBuilder};

use super::fonts::BUNDLED;

/// Horizontal scan lines across the band: the band is about a stroke's width tall, so
/// this many lines can't step over any of a glyph's ink.
const SCANS: usize = 6;
/// Line segments per curve.
const CURVE_STEPS: usize = 8;

/// Ink spans by (face, glyph, band bottom, band top), in font units.
type SpanCache = HashMap<(usize, u32, i32, i32), Rc<Vec<(f32, f32)>>>;

thread_local! {
    static FACES: Vec<Option<Face<'static>>> = BUNDLED.into_iter().map(|b| Face::parse(b, 0).ok()).collect();
    static SPANS: RefCell<SpanCache> = RefCell::new(HashMap::new());
}

/// A bundled face's units per em, if it parsed.
pub fn units_per_em(face: usize) -> Option<f32> {
    FACES.with(|f| f.get(face)?.as_ref().map(|f| f.units_per_em() as f32))
}

/// The regular face's x-height, in ems (`vertical-align: middle` centres on half of it).
pub fn x_height_em() -> f32 {
    FACES.with(|f| {
        f.first()
            .and_then(|f| f.as_ref())
            .and_then(|f| Some(f.x_height()? as f32 / f.units_per_em() as f32))
            .unwrap_or(0.5)
    })
}

/// The x-ranges (font units, from the glyph's origin) where glyph `glyph` of bundled
/// face `face` has ink between heights `lo` and `hi` (font units, y up, `lo < hi`).
pub fn spans(face: usize, glyph: u32, lo: f32, hi: f32) -> Rc<Vec<(f32, f32)>> {
    let key = (face, glyph, lo.round() as i32, hi.round() as i32);
    if let Some(hit) = SPANS.with(|c| c.borrow().get(&key).cloned()) {
        return hit;
    }
    let found = FACES.with(|f| {
        let Some(Some(face)) = f.get(face) else { return Vec::new() };
        let Ok(id) = u16::try_from(glyph) else { return Vec::new() };
        let mut outline = Segments::default();
        if face.outline_glyph(GlyphId(id), &mut outline).is_none() {
            return Vec::new();
        }
        band_spans(&outline.lines, lo, hi)
    });
    let found = Rc::new(found);
    SPANS.with(|c| c.borrow_mut().insert(key, found.clone()));
    found
}

/// The union, over scan lines across the band, of where the outline is filled
/// (TrueType's non-zero rule).
fn band_spans(lines: &[(f32, f32, f32, f32)], lo: f32, hi: f32) -> Vec<(f32, f32)> {
    let mut spans: Vec<(f32, f32)> = Vec::new();
    for k in 0..SCANS {
        let y = lo + (k as f32 + 0.5) * (hi - lo) / SCANS as f32;
        let mut crossings: Vec<(f32, i32)> = lines
            .iter()
            .filter_map(|&(x0, y0, x1, y1)| {
                let (going_up, (ya, yb)) = if y1 > y0 { (true, (y0, y1)) } else { (false, (y1, y0)) };
                if y < ya || y >= yb || ya == yb {
                    return None;
                }
                let x = x0 + (y - y0) * (x1 - x0) / (y1 - y0);
                Some((x, if going_up { 1 } else { -1 }))
            })
            .collect();
        crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut winding = 0;
        let mut start = 0.0;
        for (x, dir) in crossings {
            let was = winding;
            winding += dir;
            if was == 0 && winding != 0 {
                start = x;
            } else if was != 0 && winding == 0 {
                spans.push((start, x));
            }
        }
    }
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged: Vec<(f32, f32)> = Vec::new();
    for (a, b) in spans {
        match merged.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => merged.push((a, b)),
        }
    }
    merged
}

/// A glyph outline flattened to line segments `(x0, y0, x1, y1)`.
#[derive(Default)]
struct Segments {
    lines: Vec<(f32, f32, f32, f32)>,
    start: (f32, f32),
    at: (f32, f32),
}

impl Segments {
    fn line(&mut self, x: f32, y: f32) {
        self.lines.push((self.at.0, self.at.1, x, y));
        self.at = (x, y);
    }
}

impl OutlineBuilder for Segments {
    fn move_to(&mut self, x: f32, y: f32) {
        self.start = (x, y);
        self.at = (x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.line(x, y);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (x0, y0) = self.at;
        for i in 1..=CURVE_STEPS {
            let t = i as f32 / CURVE_STEPS as f32;
            let u = 1.0 - t;
            self.line(u * u * x0 + 2.0 * u * t * x1 + t * t * x, u * u * y0 + 2.0 * u * t * y1 + t * t * y);
        }
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (x0, y0) = self.at;
        for i in 1..=CURVE_STEPS {
            let t = i as f32 / CURVE_STEPS as f32;
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            self.line(a * x0 + b * x1 + c * x2 + d * x, a * y0 + b * y1 + c * y2 + d * y);
        }
    }
    fn close(&mut self) {
        if self.at != self.start {
            let (x, y) = self.start;
            self.line(x, y);
        }
    }
}

/// `line` (x from `a` to `b`) with `gaps` taken out.
pub fn cut(a: f32, b: f32, gaps: &[(f32, f32)]) -> Vec<(f32, f32)> {
    let mut pieces = vec![(a, b)];
    for &(ga, gb) in gaps {
        pieces = pieces
            .into_iter()
            .flat_map(|(pa, pb)| {
                if gb <= pa || ga >= pb {
                    vec![(pa, pb)]
                } else {
                    [(pa, ga.max(pa)), (gb.min(pb), pb)].into_iter().filter(|(s, e)| e > s).collect()
                }
            })
            .collect();
    }
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glyph(c: char) -> u32 {
        FACES.with(|f| f[0].as_ref().unwrap().glyph_index(c).unwrap().0 as u32)
    }

    #[test]
    fn descenders_cross_an_underline_and_x_height_letters_do_not() {
        // JetBrains Mono: 1000 units per em; an underline about 100-170 units below the
        // baseline.
        let (lo, hi) = (-170.0, -100.0);
        for c in ['g', 'j', 'p', 'q', 'y'] {
            assert!(!spans(0, glyph(c), lo, hi).is_empty(), "{c} should have ink under the baseline");
        }
        for c in ['a', 'e', 'o', 'x', 'n'] {
            assert!(spans(0, glyph(c), lo, hi).is_empty(), "{c} has no descender");
        }
    }

    #[test]
    fn a_p_crosses_only_at_its_stem() {
        // "p"'s descender is its left stem: the ink is a narrow span at the left, not the
        // glyph's whole width.
        let s = spans(0, glyph('p'), -170.0, -100.0);
        assert_eq!(s.len(), 1);
        let (a, b) = s[0];
        assert!(b - a < 200.0 && a < 300.0, "{s:?}");
    }

    #[test]
    fn cut_removes_gaps_from_a_line() {
        assert_eq!(cut(0.0, 10.0, &[(2.0, 3.0), (5.0, 6.0)]), vec![(0.0, 2.0), (3.0, 5.0), (6.0, 10.0)]);
        assert_eq!(cut(0.0, 10.0, &[(-1.0, 1.0), (9.0, 12.0)]), vec![(1.0, 9.0)]);
        assert_eq!(cut(0.0, 10.0, &[(-1.0, 11.0)]), Vec::<(f32, f32)>::new());
    }
}
