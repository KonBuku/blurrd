//! Straight panel edges: the colour-independent structure the screen
//! surfaces share.
//!
//! Every screen surface is a panel with long, perfectly straight edges at
//! fixed reference positions: the outline of the scoreboard, the bands
//! of the death recap, the plate of the "Spectating" banner. Gameplay
//! has edges too, but not long straight ones exactly there, and textured
//! walls change on both sides of any line. An edge cell therefore counts
//! only if the step across the line is large ([`MIN_STEP`]) and at least
//! twice the change between the cells two steps further out on one side
//! of the line (the panel interior or the background behind it is smooth
//! there). The score of an edge is the fraction of its cells that count.
//!
//! Only the step size matters, not its direction or colour, so team
//! colours, colour-blind modes and GPU colour filters do not change the
//! structure; a darkening filter shrinks every step, which the
//! thresholds of the gates leave room for.

use super::sample::RefPlane;

/// Smallest colour step (any channel, 8-bit) across a panel edge.
pub const MIN_STEP: i16 = 10;

/// The largest channel difference of two cells.
fn diff(a: [i16; 3], b: [i16; 3]) -> i16 {
    (a[0] - b[0])
        .abs()
        .max((a[1] - b[1]).abs())
        .max((a[2] - b[2]).abs())
}

/// Whether the step between `inner` (the cell after the line) and
/// `outer` (the cell before it) is an edge, given the cells beyond them
/// on each side.
pub(super) fn is_edge(before: [i16; 3], outer: [i16; 3], inner: [i16; 3], after: [i16; 3]) -> bool {
    let step = diff(inner, outer);
    let side = diff(outer, before).min(diff(after, inner));
    step >= MIN_STEP && step >= 2 * side
}

/// Widest thin line (a card outline, the spectating bar) in reference
/// pixels: one source pixel spans up to three reference pixels in a 720p
/// recording.
pub const LINE_W: i32 = 3;

/// Whether the vertical line before plane column `x` is an edge in row
/// `y`.
pub(super) fn edge_at(plane: &RefPlane, x: i32, y: i32) -> bool {
    is_edge(
        plane.at(x - 2, y),
        plane.at(x - 1, y),
        plane.at(x, y),
        plane.at(x + 1, y),
    )
}

/// Whether a thin vertical line starts at plane column `x` in row `y`:
/// edges on both of its sides, at most [`LINE_W`] cells apart. A step
/// between two textures (card art, a wall) or a wide post has only one.
pub(super) fn line_at(plane: &RefPlane, x: i32, y: i32) -> bool {
    edge_at(plane, x, y) && (1..=LINE_W).any(|w| edge_at(plane, x + w, y))
}

/// Score of a vertical edge at reference x `x` over reference rows
/// `y0..y1`, the best over line positions within `search` cells.
///
/// Returns the score and the line's offset in reference pixels.
pub fn vertical(plane: &RefPlane, x: i32, y0: i32, y1: i32, search: i32) -> (f32, i32) {
    let (cx, cy0) = plane.cell(x, y0);
    let (_, cy1) = plane.cell(x, y1);
    let rows = (cy1 - cy0).max(1);
    let mut best = (0.0, 0);
    for d in -search..=search {
        let c = cx + d;
        let hits = (cy0..cy1)
            .filter(|&cy| {
                is_edge(
                    plane.at(c - 2, cy),
                    plane.at(c - 1, cy),
                    plane.at(c, cy),
                    plane.at(c + 1, cy),
                )
            })
            .count();
        let score = hits as f32 / rows as f32;
        if score > best.0 {
            best = (score, d * plane.step());
        }
    }
    best
}

/// Edge cells of the horizontal line between cell rows `cy - 1` and `cy`
/// in columns `cx0..cx1` (cells outside the plane are black).
fn horizontal_at(plane: &RefPlane, cy: i32, cx0: i32, cx1: i32) -> f32 {
    let rows = [cy - 2, cy - 1, cy, cy + 1].map(|y| plane.row(y));
    if let [Some(before), Some(outer), Some(inner), Some(after)] = rows {
        let (lo, hi) = (cx0.max(0) as usize, (cx1.max(0) as usize).min(before.len()));
        if lo < hi {
            let hits = before[lo..hi]
                .iter()
                .zip(&outer[lo..hi])
                .zip(&inner[lo..hi])
                .zip(&after[lo..hi])
                .filter(|&(((b, o), i), a)| is_edge(*b, *o, *i, *a))
                .count();
            return hits as f32;
        }
    }
    (cx0..cx1)
        .filter(|&cx| {
            is_edge(
                plane.at(cx, cy - 2),
                plane.at(cx, cy - 1),
                plane.at(cx, cy),
                plane.at(cx, cy + 1),
            )
        })
        .count() as f32
}

/// The score of the horizontal edge exactly at reference y `y` over
/// reference columns `x0..x1` (no search).
pub fn horizontal_at_y(plane: &RefPlane, y: i32, x0: i32, x1: i32) -> f32 {
    let (cx0, cy) = plane.cell(x0, y);
    let (cx1, _) = plane.cell(x1, y);
    horizontal_at(plane, cy, cx0, cx1) / (cx1 - cx0).max(1) as f32
}

/// The mean score of several horizontal edges that move together (a
/// panel sliding vertically), at the best common offset within `search`
/// cells; each edge may lie one cell off that offset. Each edge is
/// `(y, x0, x1)` in reference pixels. Keep `search` below the smallest
/// distance between the edges, or a shifted set can match itself.
///
/// Returns the score and the common offset in reference pixels.
pub fn horizontal_set(plane: &RefPlane, edges: &[(i32, i32, i32)], search: i32) -> (f32, i32) {
    let mut best = (0.0, 0);
    if edges.is_empty() {
        return best;
    }
    // Edge counts of every line at every offset it can take, computed
    // once: `counts[line][d + search + 1]` for d in -search-1..=search+1.
    let window = (2 * search + 3) as usize;
    let counts: Vec<Vec<f32>> = edges
        .iter()
        .map(|&(y, x0, x1)| {
            let (cx0, cy) = plane.cell(x0, y);
            let (cx1, _) = plane.cell(x1, y);
            let n = (cx1 - cx0).max(1) as f32;
            (0..window as i32)
                .map(|i| horizontal_at(plane, cy + i - search - 1, cx0, cx1) / n)
                .collect()
        })
        .collect();
    for d in -search..=search {
        let i = (d + search + 1) as usize;
        // Each line may sit one cell off the common offset: a line
        // between two reference rows falls inside a gate cell, and a
        // one-pixel separator has an edge on both of its sides.
        let total: f32 = counts
            .iter()
            .map(|c| c[i - 1].max(c[i]).max(c[i + 1]))
            .sum();
        let score = total / edges.len() as f32;
        if score > best.0 {
            best = (score, d * plane.step());
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use blurrd_core::RectI;

    use super::{horizontal_set, vertical};
    use crate::image::RgbView;
    use crate::screens::sample::{Anchor, FrameView, RefPlane, ScreenMap};

    /// A 64 × 48 frame: a flat panel (value 120) over a flat background
    /// (40) from x 16 and y 12, or over checkerboard texture.
    fn frame(textured: bool) -> Vec<u8> {
        let mut data = vec![];
        for y in 0..48 {
            for x in 0..64 {
                let v = if x >= 16 && y >= 12 {
                    120
                } else if textured && (x + y) % 2 == 0 {
                    90
                } else {
                    40
                };
                data.extend([v, v, v]);
            }
        }
        data
    }

    fn plane(data: &[u8]) -> RefPlane {
        let view = FrameView::native(RgbView::new(data, 64, 48, 192).unwrap());
        let mut p = RefPlane::new();
        p.sample(
            &view,
            &ScreenMap::REFERENCE,
            Anchor::Left,
            RectI::new(0, 0, 64, 48).unwrap(),
            1,
        );
        p
    }

    #[test]
    fn finds_straight_edges_and_their_offset() {
        let p = plane(&frame(false));
        assert_eq!(vertical(&p, 15, 14, 46, 2), (1.0, 1));
        assert_eq!(horizontal_set(&p, &[(12, 18, 62)], 0), (1.0, 0));
        assert_eq!(horizontal_set(&p, &[(8, 18, 62)], 6), (1.0, 3));
        assert!(vertical(&p, 40, 14, 46, 2).0 < 0.01);
    }

    #[test]
    fn texture_on_one_side_still_shows_the_edge_but_not_a_false_one() {
        let p = plane(&frame(true));
        // The panel interior is smooth, so its edge is found ...
        assert!(vertical(&p, 16, 14, 46, 0).0 > 0.9);
        // ... but the checkerboard alone has no edge.
        assert!(vertical(&p, 8, 0, 10, 2).0 < 0.01);
    }
}
