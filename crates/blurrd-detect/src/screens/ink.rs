//! Does a name slot show ink?
//!
//! Screen surfaces blur the whole slot of every name that is there
//! (Safe geometry, plan 7.3), so the per-slot question is only whether
//! the slot holds text at all: an empty scoreboard row (fewer players)
//! or a combat-report row that is not shown must stay unblurred. Text is
//! cells that differ clearly from the slot's typical colour, in any
//! direction (white names, dim grey names of dead players, dark text on
//! a light filter), spread over several columns. Colour does not matter.

use blurrd_core::RectI;

use super::sample::RefPlane;

/// Smallest luminance difference from the slot's median that counts as
/// ink (dim names of dead players differ by about 60 on real
/// scoreboards).
pub const INK_CONTRAST: i16 = 28;
/// Fewest ink cells a slot needs.
pub const INK_MIN_CELLS: usize = 10;
/// Fewest distinct ink columns a slot needs.
pub const INK_MIN_COLUMNS: usize = 5;

/// Reusable buffers of the ink test.
#[derive(Debug, Clone, Default)]
pub(super) struct InkScratch {
    lumas: Vec<i16>,
    sorted: Vec<i16>,
    columns: Vec<bool>,
}

/// Whether the reference rectangle `rect` of `plane` shows ink.
pub(super) fn has_ink(plane: &RefPlane, rect: RectI, scratch: &mut InkScratch) -> bool {
    let (cx0, cy0) = plane.cell(rect.x0, rect.y0);
    let (cx1, cy1) = plane.cell(rect.x1, rect.y1);
    if cx1 <= cx0 || cy1 <= cy0 {
        return false;
    }
    scratch.lumas.clear();
    for cy in cy0..cy1 {
        for cx in cx0..cx1 {
            scratch.lumas.push(plane.luma(cx, cy));
        }
    }
    scratch.sorted.clone_from(&scratch.lumas);
    let mid = scratch.sorted.len() / 2;
    let median = *scratch.sorted.select_nth_unstable(mid).1;
    let width = (cx1 - cx0) as usize;
    scratch.columns.clear();
    scratch.columns.resize(width, false);
    let mut cells = 0;
    for (i, &l) in scratch.lumas.iter().enumerate() {
        if (l - median).abs() >= INK_CONTRAST {
            cells += 1;
            scratch.columns[i % width] = true;
        }
    }
    cells >= INK_MIN_CELLS && scratch.columns.iter().filter(|&&c| c).count() >= INK_MIN_COLUMNS
}

#[cfg(test)]
mod tests {
    use blurrd_core::RectI;

    use super::{InkScratch, has_ink};
    use crate::image::RgbView;
    use crate::screens::sample::{Anchor, FrameView, RefPlane, ScreenMap};

    #[test]
    fn finds_text_but_not_flat_or_tiny_marks() {
        let (w, h) = (40u32, 12u32);
        let mut data = vec![90u8; (3 * w * h) as usize];
        // A dot of 3 × 3 cells.
        for y in 2..5 {
            for x in 2..5 {
                let i = 3 * (y * w + x) as usize;
                data[i..i + 3].copy_from_slice(&[200; 3]);
            }
        }
        let view = FrameView::native(RgbView::new(&data, w, h, 3 * w as usize).unwrap());
        let mut plane = RefPlane::new();
        let all = RectI::new(0, 0, w as i32, h as i32).unwrap();
        plane.sample(&view, &ScreenMap::REFERENCE, Anchor::Left, all, 1);
        let mut scratch = InkScratch::default();
        assert!(!has_ink(&plane, all, &mut scratch));
        // A dim stroke over 20 columns.
        for x in 10..30 {
            for y in 6..8 {
                let i = 3 * (y * w + x) as usize;
                data[i..i + 3].copy_from_slice(&[150; 3]);
            }
        }
        let view = FrameView::native(RgbView::new(&data, w, h, 3 * w as usize).unwrap());
        plane.sample(&view, &ScreenMap::REFERENCE, Anchor::Left, all, 1);
        assert!(has_ink(&plane, all, &mut scratch));
    }
}
