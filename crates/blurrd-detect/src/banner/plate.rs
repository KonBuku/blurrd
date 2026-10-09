//! The grey plate under a banner name (port spec section 10, item 4).
//!
//! The name of an ACE or clutch banner is always written on a plate: a
//! translucent light band from y 298 to 328 with straight top and bottom
//! edges, about 390 px wide and centred on the name. Plate and name slide
//! in together from the right. Other white text in the same place, the
//! "X ULT READY" notification above all, has no plate, so a name counts
//! only when a plate lies under it (the plate gate).
//!
//! The plate is found from its two horizontal edges alone: in every column
//! the rows just inside each edge differ from the rows just outside it,
//! with the same sign along the plate. The sign depends on the background
//! (the plate lightens dark scenes and darkens a white sky), the edges do
//! not depend on colour, so colour-blind palettes and hue or saturation
//! filters do not change them. The edge threshold follows the frame's
//! white level like the name thresholds.

use blurrd_core::RectI;

use crate::image::RgbView;
use crate::killfeed::FEED_ROI;

/// Rows just inside (`.0`) and just outside (`.1`) the plate's top edge
/// at y 298.
const TOP_ROWS: [(i32, i32); 2] = [(299, 302), (294, 297)];
/// Rows just inside (`.0`) and just outside (`.1`) the plate's bottom
/// edge at y 328.
const BOTTOM_ROWS: [(i32, i32); 2] = [(325, 328), (329, 332)];
/// Columns searched for the plate: from the strip's left end to the frame
/// edge (a sliding plate reaches far to the right).
const SEARCH_X: (i32, i32) = (690, 1920);
/// An edge needs at least this mean difference between its inside and
/// outside rows (at the tuned white level). On 1,816 settled frames of 7
/// recorded banners (a white sky and heavy ability effects included) the
/// median edge differs by 13 to 143 levels, and both edges reach 8 in
/// 99.5 % of the columns under the name (at least 78 % in every frame).
pub(super) const EDGE_MIN: u8 = 8;
/// Plate columns of one plate may be separated by at most this many other
/// columns (the background shows through in places).
const MAX_GAP: i32 = 8;
/// A plate is at least this wide. The real plate is about 390 px wide and
/// enters the frame from the right edge, so this only rejects short
/// horizontal structures of the scene.
const MIN_WIDTH: i32 = 150;
/// At least this share of a plate's columns show both edges.
const MIN_DENSITY_PERCENT: i32 = 60;
/// A name lies on a plate when the plate covers at least this share of the
/// name's width.
const NAME_COVER_PERCENT: i32 = 75;

/// The plate found in one frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Plate {
    /// Left end of the plate's detected edges.
    pub x0: i32,
    /// Right end (exclusive) of the plate's detected edges.
    pub x1: i32,
}

impl Plate {
    /// Whether a name with the text bounds `text` lies on the plate: the
    /// plate covers at least 75 % of its width.
    pub fn carries(&self, text: &RectI) -> bool {
        let overlap = self.x1.min(text.x1) - self.x0.max(text.x0);
        100 * overlap >= NAME_COVER_PERCENT * (text.x1 - text.x0)
    }
}

/// Reusable buffers.
#[derive(Debug, Clone, Default)]
pub(super) struct PlateScratch {
    /// Per column of [`SEARCH_X`]: the sign pattern of both edges
    /// (`0` = not a plate column).
    pattern: Vec<i8>,
}

/// The mean of `min(R, G, B)` over rows `y0..y1` of the crop column at
/// reference `x`.
fn column_mean(crop: &RgbView<'_>, x: i32, (y0, y1): (i32, i32)) -> i32 {
    let sum: i32 = (y0..y1)
        .map(|y| {
            let rgb = crop.pixel((x - FEED_ROI.x0) as u32, (y - FEED_ROI.y0) as u32);
            i32::from(rgb[0].min(rgb[1]).min(rgb[2]))
        })
        .sum();
    sum / (y1 - y0)
}

/// The sign of an edge's difference, or `0` below the threshold.
fn edge_sign(rows: [(i32, i32); 2], crop: &RgbView<'_>, x: i32, min: i32) -> i8 {
    let d = column_mean(crop, x, rows[0]) - column_mean(crop, x, rows[1]);
    if d >= min {
        1
    } else if d <= -min {
        -1
    } else {
        0
    }
}

/// Finds the widest plate in `crop` (the [`FEED_ROI`] of a frame) with
/// the edge threshold `edge_min`.
pub(super) fn find_plate(crop: &RgbView<'_>, edge_min: u8, s: &mut PlateScratch) -> Option<Plate> {
    let min = i32::from(edge_min);
    s.pattern.clear();
    s.pattern.extend((SEARCH_X.0..SEARCH_X.1).map(|x| {
        let top = edge_sign(TOP_ROWS, crop, x, min);
        let bottom = edge_sign(BOTTOM_ROWS, crop, x, min);
        // Four patterns (top and bottom each lighter or darker inside),
        // 0 where an edge is missing.
        if top == 0 || bottom == 0 {
            0
        } else {
            2 * top + bottom
        }
    }));
    let mut best: Option<(Plate, i32)> = None;
    let mut run: Option<(i8, i32, i32, i32)> = None; // pattern, first, last, columns
    let mut close = |run: Option<(i8, i32, i32, i32)>| {
        if let Some((_, first, last, columns)) = run {
            let width = last + 1 - first;
            if width >= MIN_WIDTH
                && 100 * columns >= MIN_DENSITY_PERCENT * width
                && best.is_none_or(|(_, w)| width > w)
            {
                best = Some((
                    Plate {
                        x0: first + SEARCH_X.0,
                        x1: last + 1 + SEARCH_X.0,
                    },
                    width,
                ));
            }
        }
    };
    for (i, &p) in s.pattern.iter().enumerate() {
        let i = i as i32;
        if run.is_some_and(|(_, _, last, _)| i - last > MAX_GAP) {
            close(run.take());
        }
        // Columns without both edges, or with another pattern, are gaps
        // of the open plate.
        run = match run {
            Some((q, first, _, columns)) if q == p => Some((q, first, i, columns + 1)),
            None if p != 0 => Some((p, i, i, 1)),
            other => other,
        };
    }
    close(run);
    best.map(|(plate, _)| plate)
}
