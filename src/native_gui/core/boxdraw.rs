//! Box drawing (U+2500-U+257F) as exact cell geometry, the way terminals draw it.
//!
//! The WebView draws these characters from whatever font the browser falls back to for
//! them (the web's JetBrains Mono subset leaves them out). That font's columns are a
//! little narrower or wider than JetBrains Mono's, so a box's edges drift off its corners
//! along a row (worst on Windows: Cascadia Code is 2% narrower), and its glyphs are
//! designed for taller lines than the output's one-em rows, so vertical lines break
//! between rows. Drawn here instead, each character fills exactly one column and one
//! row, and every line meets its neighbour's at the cell edge.
//!
//! Everything is computed in device pixels so lines land on whole pixels: a line of
//! weight w that leaves a cell's edge sits at exactly the place the next cell's line of
//! the same weight enters.

/// How heavy one arm of a character is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Weight {
    None,
    Light,
    Heavy,
    Double,
}

use Weight::{Double as D, Heavy as H, Light as L, None as N};

/// The four arms of a line character: left, right, up, down.
pub type Arms = [Weight; 4];

/// What a character is made of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    Lines(Arms),
    /// A dashed line: horizontal or not, heavy or not, and how many dashes.
    Dashes { horizontal: bool, heavy: bool, count: u8 },
    /// A rounded corner: which way its two arms go (right or left, down or up).
    Arc { right: bool, down: bool },
    /// `╱`, `╲` or `╳`.
    Diagonal { rising: bool, falling: bool },
}

pub fn is_box_drawing(c: char) -> bool {
    ('\u{2500}'..='\u{257F}').contains(&c)
}

/// U+2500-U+254B, in order: left, right, up, down.
const LINES: [Arms; 76] = [
    [L, L, N, N], [H, H, N, N], [N, N, L, L], [N, N, H, H], // ─ ━ │ ┃
    [N; 4], [N; 4], [N; 4], [N; 4], [N; 4], [N; 4], [N; 4], [N; 4], // ┄-┋ (dashes)
    [N, L, N, L], [N, H, N, L], [N, L, N, H], [N, H, N, H], // ┌ ┍ ┎ ┏
    [L, N, N, L], [H, N, N, L], [L, N, N, H], [H, N, N, H], // ┐ ┑ ┒ ┓
    [N, L, L, N], [N, H, L, N], [N, L, H, N], [N, H, H, N], // └ ┕ ┖ ┗
    [L, N, L, N], [H, N, L, N], [L, N, H, N], [H, N, H, N], // ┘ ┙ ┚ ┛
    [N, L, L, L], [N, H, L, L], [N, L, H, L], [N, L, L, H], // ├ ┝ ┞ ┟
    [N, L, H, H], [N, H, H, L], [N, H, L, H], [N, H, H, H], // ┠ ┡ ┢ ┣
    [L, N, L, L], [H, N, L, L], [L, N, H, L], [L, N, L, H], // ┤ ┥ ┦ ┧
    [L, N, H, H], [H, N, H, L], [H, N, L, H], [H, N, H, H], // ┨ ┩ ┪ ┫
    [L, L, N, L], [H, L, N, L], [L, H, N, L], [H, H, N, L], // ┬ ┭ ┮ ┯
    [L, L, N, H], [H, L, N, H], [L, H, N, H], [H, H, N, H], // ┰ ┱ ┲ ┳
    [L, L, L, N], [H, L, L, N], [L, H, L, N], [H, H, L, N], // ┴ ┵ ┶ ┷
    [L, L, H, N], [H, L, H, N], [L, H, H, N], [H, H, H, N], // ┸ ┹ ┺ ┻
    [L, L, L, L], [H, L, L, L], [L, H, L, L], [H, H, L, L], // ┼ ┽ ┾ ┿
    [L, L, H, L], [L, L, L, H], [L, L, H, H], [H, L, H, L], // ╀ ╁ ╂ ╃
    [L, H, H, L], [H, L, L, H], [L, H, L, H], [H, H, H, L], // ╄ ╅ ╆ ╇
    [H, H, L, H], [H, L, H, H], [L, H, H, H], [H, H, H, H], // ╈ ╉ ╊ ╋
];

/// U+2550-U+256C.
const DOUBLES: [Arms; 29] = [
    [D, D, N, N], [N, N, D, D], // ═ ║
    [N, D, N, L], [N, L, N, D], [N, D, N, D], // ╒ ╓ ╔
    [D, N, N, L], [L, N, N, D], [D, N, N, D], // ╕ ╖ ╗
    [N, D, L, N], [N, L, D, N], [N, D, D, N], // ╘ ╙ ╚
    [D, N, L, N], [L, N, D, N], [D, N, D, N], // ╛ ╜ ╝
    [N, D, L, L], [N, L, D, D], [N, D, D, D], // ╞ ╟ ╠
    [D, N, L, L], [L, N, D, D], [D, N, D, D], // ╡ ╢ ╣
    [D, D, N, L], [L, L, N, D], [D, D, N, D], // ╤ ╥ ╦
    [D, D, L, N], [L, L, D, N], [D, D, D, N], // ╧ ╨ ╩
    [D, D, L, L], [L, L, D, D], [D, D, D, D], // ╪ ╫ ╬
];

/// U+2574-U+257F: half lines.
const HALVES: [Arms; 12] = [
    [L, N, N, N], [N, N, L, N], [N, L, N, N], [N, N, N, L], // ╴ ╵ ╶ ╷
    [H, N, N, N], [N, N, H, N], [N, H, N, N], [N, N, N, H], // ╸ ╹ ╺ ╻
    [L, H, N, N], [N, N, L, H], [H, L, N, N], [N, N, H, L], // ╼ ╽ ╾ ╿
];

pub fn glyph(c: char) -> Option<Glyph> {
    let n = c as u32;
    Some(match n {
        0x2504..=0x250B => {
            let k = n - 0x2504;
            Glyph::Dashes { horizontal: k % 4 < 2, heavy: k % 2 == 1, count: if k < 4 { 3 } else { 4 } }
        }
        0x254C..=0x254F => {
            let k = n - 0x254C;
            Glyph::Dashes { horizontal: k < 2, heavy: k % 2 == 1, count: 2 }
        }
        0x2500..=0x254B => Glyph::Lines(LINES[(n - 0x2500) as usize]),
        0x2550..=0x256C => Glyph::Lines(DOUBLES[(n - 0x2550) as usize]),
        0x256D => Glyph::Arc { right: true, down: true },
        0x256E => Glyph::Arc { right: false, down: true },
        0x256F => Glyph::Arc { right: false, down: false },
        0x2570 => Glyph::Arc { right: true, down: false },
        0x2571 => Glyph::Diagonal { rising: true, falling: false },
        0x2572 => Glyph::Diagonal { rising: false, falling: true },
        0x2573 => Glyph::Diagonal { rising: true, falling: true },
        0x2574..=0x257F => Glyph::Lines(HALVES[(n - 0x2574) as usize]),
        _ => return None,
    })
}

/// A cell in device pixels: `x0..x1` by `y0..y1`, whole numbers.
#[derive(Clone, Copy, Debug)]
pub struct CellPx {
    pub x0: f32,
    pub x1: f32,
    pub y0: f32,
    pub y1: f32,
}

/// One part of a stroked path, in device pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seg {
    Move(f32, f32),
    Line(f32, f32),
    /// Two control points, then the end point.
    Cubic([f32; 6]),
}

/// What to paint for a character, in device pixels.
#[derive(Debug, Default)]
pub struct Shape {
    /// Filled rectangles `[x0, y0, x1, y1]`.
    pub rects: Vec<[f32; 4]>,
    /// Stroked paths, with their width.
    pub strokes: Vec<(f32, Vec<Seg>)>,
}

/// Line thicknesses for a row this many device pixels tall.
#[derive(Clone, Copy, Debug)]
pub struct Thickness {
    pub light: f32,
    pub heavy: f32,
}

impl Thickness {
    pub fn for_row(row_px: f32) -> Self {
        let light = (row_px * 0.075).round().max(1.0);
        Thickness { light, heavy: (row_px * 0.16).round().max(light + 1.0) }
    }
}

/// The strips a line of `weight` occupies across its direction, centred on `centre`:
/// one, or two for a double line (each `light` thick, `light` apart). Ascending.
fn bands(centre: f32, weight: Weight, t: Thickness) -> Vec<(f32, f32)> {
    let one = |w: f32| {
        let s = centre - (w / 2.0).floor();
        (s, s + w)
    };
    match weight {
        Weight::None => Vec::new(),
        Weight::Light => vec![one(t.light)],
        Weight::Heavy => vec![one(t.heavy)],
        Weight::Double => {
            let span = 3.0 * t.light;
            let s = centre - (span / 2.0).floor();
            vec![(s, s + t.light), (s + 2.0 * t.light, s + span)]
        }
    }
}

/// How far toward the centre line `k` of an arm reaches: its start for an arm on the
/// plus side (right, down), its end for one on the minus side (left, up). `minus` and
/// `plus` are the bands of the perpendicular arms on either side of this arm's lines
/// (up and down for a horizontal arm), given along this arm's direction.
fn reach(
    plus_side: bool,
    double: bool,
    k: usize,
    minus: &[(f32, f32)],
    plus: &[(f32, f32)],
    opposite: bool,
    centre: f32,
) -> f32 {
    let start = |b: (f32, f32)| if plus_side { b.0 } else { b.1 };
    // A double line's own side decides: its line k=0 is on the minus side.
    if double {
        let (same, other) = if k == 0 { (minus, plus) } else { (plus, minus) };
        // Meeting a double line on its own side, it stops at the nearer of the two (the
        // inner corner); meeting a single one, it joins it; with only the other side's
        // arm, it runs to that arm's far line (the outer corner).
        let near = |b: &[(f32, f32)]| if plus_side { b[b.len() - 1] } else { b[0] };
        let far = |b: &[(f32, f32)]| if plus_side { b[0] } else { b[b.len() - 1] };
        return if !same.is_empty() {
            start(near(same))
        } else if !other.is_empty() {
            start(far(other))
        } else {
            centre
        };
    }
    if opposite {
        return centre;
    }
    let mut all: Vec<(f32, f32)> = minus.iter().chain(plus).copied().collect();
    all.sort_by(|a, b| a.0.total_cmp(&b.0));
    if all.is_empty() {
        return centre;
    }
    let near = if plus_side { all[all.len() - 1] } else { all[0] };
    let far = if plus_side { all[0] } else { all[all.len() - 1] };
    let crossing = !minus.is_empty() && !plus.is_empty();
    // A tee off a double line touches its near line; a corner spans the whole width of
    // the line it turns into.
    if crossing && (minus.len() == 2 || plus.len() == 2) { start(near) } else { start(far) }
}

/// The shape of box-drawing character `c` in `cell`.
pub fn shape(c: char, cell: CellPx, t: Thickness) -> Option<Shape> {
    let g = glyph(c)?;
    let CellPx { x0, x1, y0, y1 } = cell;
    let cx = x0 + ((x1 - x0) / 2.0).floor();
    let cy = y0 + ((y1 - y0) / 2.0).floor();
    let mut out = Shape::default();
    match g {
        Glyph::Lines([left, right, up, down]) => {
            let h_left = bands(cy, left, t);
            let h_right = bands(cy, right, t);
            let v_up = bands(cx, up, t);
            let v_down = bands(cx, down, t);
            let some = |w: Weight| w != Weight::None;
            for (k, &(a, b)) in h_left.iter().enumerate() {
                let end = reach(false, left == D, k, &v_up, &v_down, some(right), cx);
                out.rects.push([x0, a, end, b]);
            }
            for (k, &(a, b)) in h_right.iter().enumerate() {
                let start = reach(true, right == D, k, &v_up, &v_down, some(left), cx);
                out.rects.push([start, a, x1, b]);
            }
            for (k, &(a, b)) in v_up.iter().enumerate() {
                let end = reach(false, up == D, k, &h_left, &h_right, some(down), cy);
                out.rects.push([a, y0, b, end]);
            }
            for (k, &(a, b)) in v_down.iter().enumerate() {
                let start = reach(true, down == D, k, &h_left, &h_right, some(up), cy);
                out.rects.push([a, start, b, y1]);
            }
            out.rects.retain(|r| r[2] > r[0] && r[3] > r[1]);
        }
        Glyph::Dashes { horizontal, heavy, count } => {
            let (lo, hi) = if horizontal { (x0, x1) } else { (y0, y1) };
            let band = bands(if horizontal { cy } else { cx }, if heavy { H } else { L }, t)[0];
            let seg = (hi - lo) / count as f32;
            let gap = (seg * 0.35).round().max(1.0);
            for i in 0..count {
                let a = (lo + seg * i as f32 + gap / 2.0).round();
                let b = (lo + seg * (i + 1) as f32 - gap / 2.0).round();
                if b > a {
                    out.rects.push(if horizontal { [a, band.0, b, band.1] } else { [band.0, a, band.1, b] });
                }
            }
        }
        Glyph::Arc { right, down } => {
            // The line's centre is where a straight light line's centre is, so the arc
            // meets its neighbours exactly.
            let (h, v) = (bands(cy, L, t)[0], bands(cx, L, t)[0]);
            let (ly, lx) = ((h.0 + h.1) / 2.0, (v.0 + v.1) / 2.0);
            let edge_x = if right { x1 } else { x0 };
            let edge_y = if down { y1 } else { y0 };
            let r = (edge_x - lx).abs().min((edge_y - ly).abs());
            let (sx, sy) = (if right { 1.0 } else { -1.0 }, if down { 1.0 } else { -1.0 });
            const K: f32 = 0.552_284_8; // a quarter circle as one cubic
            let segs = vec![
                Seg::Move(edge_x, ly),
                Seg::Line(lx + sx * r, ly),
                Seg::Cubic([lx + sx * r * (1.0 - K), ly, lx, ly + sy * r * (1.0 - K), lx, ly + sy * r]),
                Seg::Line(lx, edge_y),
            ];
            out.strokes.push((t.light, segs));
        }
        Glyph::Diagonal { rising, falling } => {
            if rising {
                out.strokes.push((t.light, vec![Seg::Move(x0, y1), Seg::Line(x1, y0)]));
            }
            if falling {
                out.strokes.push((t.light, vec![Seg::Move(x0, y0), Seg::Line(x1, y1)]));
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CELL: CellPx = CellPx { x0: 100.0, x1: 111.0, y0: 200.0, y1: 218.0 };
    const T: Thickness = Thickness { light: 1.0, heavy: 3.0 };

    /// The strips of rectangles touching one edge of the cell: `(edge, from, to)` with
    /// edge 0 left, 1 right, 2 top, 3 bottom.
    fn at_edge(s: &Shape, edge: usize) -> Vec<(f32, f32)> {
        let mut v: Vec<(f32, f32)> = s
            .rects
            .iter()
            .filter(|r| match edge {
                0 => r[0] == CELL.x0,
                1 => r[2] == CELL.x1,
                2 => r[1] == CELL.y0,
                _ => r[3] == CELL.y1,
            })
            .map(|r| if edge < 2 { (r[1], r[3]) } else { (r[0], r[2]) })
            .collect();
        v.sort_by(|a, b| a.0.total_cmp(&b.0));
        v.dedup();
        v
    }

    #[test]
    fn every_character_in_the_block_has_a_shape() {
        for c in '\u{2500}'..='\u{257F}' {
            assert!(shape(c, CELL, T).is_some(), "{c} U+{:04X}", c as u32);
        }
        assert!(shape('a', CELL, T).is_none());
    }

    #[test]
    fn each_arm_leaves_its_edge_exactly_where_a_neighbour_of_that_weight_enters() {
        // So `─┬─` or `║╠═` join without a step or a gap: an arm's strips at the cell's
        // edge are the bands of its weight, for every line character.
        let cy = CELL.y0 + ((CELL.y1 - CELL.y0) / 2.0).floor();
        let cx = CELL.x0 + ((CELL.x1 - CELL.x0) / 2.0).floor();
        for c in '\u{2500}'..='\u{257F}' {
            let Some(Glyph::Lines(arms)) = glyph(c) else { continue };
            let s = shape(c, CELL, T).unwrap();
            for (edge, w) in arms.iter().enumerate() {
                let centre = if edge < 2 { cy } else { cx };
                assert_eq!(at_edge(&s, edge), bands(centre, *w, T), "{c} edge {edge}");
            }
        }
    }

    #[test]
    fn a_corner_is_closed_and_a_double_corner_nests() {
        // ┌: the horizontal arm starts at the vertical line's left edge.
        let s = shape('┌', CELL, T).unwrap();
        let v = s.rects.iter().find(|r| r[3] == CELL.y1).unwrap();
        let h = s.rects.iter().find(|r| r[2] == CELL.x1).unwrap();
        assert_eq!(h[0], v[0]);
        assert_eq!(v[1], h[1]);
        // ╔: the outer lines meet at the outer corner, the inner ones at the inner one.
        let s = shape('╔', CELL, T).unwrap();
        let hs: Vec<_> = s.rects.iter().filter(|r| r[2] == CELL.x1).collect();
        let vs: Vec<_> = s.rects.iter().filter(|r| r[3] == CELL.y1).collect();
        let (outer_h, inner_h) = (hs.iter().min_by(|a, b| a[1].total_cmp(&b[1])).unwrap(), hs.iter().max_by(|a, b| a[1].total_cmp(&b[1])).unwrap());
        let (outer_v, inner_v) = (vs.iter().min_by(|a, b| a[0].total_cmp(&b[0])).unwrap(), vs.iter().max_by(|a, b| a[0].total_cmp(&b[0])).unwrap());
        assert_eq!(outer_h[0], outer_v[0]);
        assert_eq!(outer_v[1], outer_h[1]);
        assert_eq!(inner_h[0], inner_v[0]);
        assert_eq!(inner_v[1], inner_h[1]);
    }

    #[test]
    fn a_double_tee_keeps_its_through_line_whole() {
        // ╦: the top line runs edge to edge; the lower one breaks for the stem.
        let s = shape('╦', CELL, T).unwrap();
        let top = bands(CELL.y0 + 9.0, D, T)[0];
        let top_cover: f32 = s.rects.iter().filter(|r| (r[1], r[3]) == top).map(|r| r[2] - r[0]).sum();
        assert!(top_cover >= CELL.x1 - CELL.x0, "{:?}", s.rects);
    }

    #[test]
    fn rows_fill_the_cell_top_to_bottom() {
        // │ and ║ reach both edges, so stacked rows draw one unbroken line.
        for c in ['│', '║', '┃', '├', '╠'] {
            let s = shape(c, CELL, T).unwrap();
            assert!(s.rects.iter().any(|r| r[1] == CELL.y0), "{c}");
            assert!(s.rects.iter().any(|r| r[3] == CELL.y1), "{c}");
        }
    }

    #[test]
    fn thickness_grows_with_the_row() {
        let t = Thickness::for_row(18.0);
        assert_eq!((t.light, t.heavy), (1.0, 3.0));
        let t = Thickness::for_row(36.0);
        assert_eq!((t.light, t.heavy), (3.0, 6.0));
    }
}
