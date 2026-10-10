//! The dark chat box behind the lines (port spec section 6, step 1).
//!
//! The box darkens the frame multiplicatively (about × 0.66) between
//! [`BOX_X`]. Its left and right edges are vertical brightness steps,
//! whatever the background: a row belongs to the box where both edges
//! are darker inside than outside.

use crate::image::RgbView;

use super::{BOX_X, BoxBand, SEARCH_Y};

/// Width of each brightness strip next to a box edge.
const STRIP: i32 = 5;
/// Strips inside the box start this far from its edge; strips outside it
/// end this far from it (the edge itself is anti-aliased).
const EDGE_GAP: i32 = 2;
/// A row is inside the box where both edge ratios are below this.
const MAX_RATIO: f32 = 0.95;
/// Added to both strip means so that black backgrounds give no ratio
/// spikes.
const RATIO_BIAS: f32 = 2.0;
/// Rows a band needs.
const MIN_ROWS: usize = 10;
/// Bands separated by at most this many rows are one box (a background
/// without edge contrast splits it).
const MERGE_GAP: i32 = 24;

/// Mean of the BT.601 grey (OpenCV's `COLOR_RGB2GRAY`, fixed point) of
/// reference columns `x0..x0 + STRIP` in one row of the crop.
fn strip_mean(row: &[[u8; 3]], x0: i32) -> f32 {
    let sum: u32 = row[x0 as usize..(x0 + STRIP) as usize]
        .iter()
        .map(|&[r, g, b]| {
            (u32::from(r) * 4899 + u32::from(g) * 9617 + u32::from(b) * 1868 + 8192) >> 14
        })
        .sum();
    sum as f32 / STRIP as f32
}

/// Measures the box edges of one frame in `crop` (the left panel;
/// reference row `y` is crop row `y - origin_y`): replaces `edges` with
/// the larger of the two edge ratios of every searched row (below
/// [`MAX_RATIO`] inside the box) and `out` with the box bands.
pub(super) fn find_bands(
    crop: &RgbView<'_>,
    origin_y: i32,
    edges: &mut Vec<f32>,
    out: &mut Vec<BoxBand>,
) {
    out.clear();
    edges.clear();
    for y in SEARCH_Y.0..SEARCH_Y.1 {
        let row = crop.row((y - origin_y) as u32).as_chunks::<3>().0;
        let inside_left = strip_mean(row, BOX_X.0 + EDGE_GAP);
        let outside_left = strip_mean(row, BOX_X.0 - EDGE_GAP - STRIP);
        let inside_right = strip_mean(row, BOX_X.1 - EDGE_GAP - STRIP);
        let outside_right = strip_mean(row, BOX_X.1 + EDGE_GAP + 1);
        let left = (inside_left + RATIO_BIAS) / (outside_left + RATIO_BIAS);
        let right = (inside_right + RATIO_BIAS) / (outside_right + RATIO_BIAS);
        edges.push(left.max(right));
    }
    let mut start = None;
    for i in 0..=edges.len() {
        let inside = edges.get(i).is_some_and(|&e| e < MAX_RATIO);
        match (inside, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                if i - s >= MIN_ROWS {
                    let run = &edges[s..i];
                    let ratio = run.iter().sum::<f32>() / run.len() as f32;
                    push_merged(
                        out,
                        BoxBand {
                            y0: SEARCH_Y.0 + s as i32,
                            y1: SEARCH_Y.0 + i as i32,
                            ratio,
                        },
                    );
                }
                start = None;
            }
            _ => {}
        }
    }
}

/// Appends `band`, merged with the previous band if the gap between them
/// is small; a merged band keeps the lower (darker) ratio.
fn push_merged(out: &mut Vec<BoxBand>, band: BoxBand) {
    match out.last_mut() {
        Some(last) if band.y0 - (last.y1 - 1) <= MERGE_GAP => {
            last.y1 = band.y1;
            last.ratio = last.ratio.min(band.ratio);
        }
        _ => out.push(band),
    }
}
