//! The dark chat box behind the lines (port spec section 6, step 1).
//!
//! The box darkens the frame multiplicatively (about × 0.66) between
//! [`BOX_X`]. Its left and right edges are vertical brightness steps,
//! whatever the background: a row belongs to the box where both edges
//! are darker inside than outside. While the player types, the open chat
//! shows its history in a taller panel whose right edge is
//! [`PANEL_X1`] (the scrollbar lies beyond it), above the input line,
//! which spans the closed box's columns: either right edge counts.
//!
//! An object whose own edge lies on a right box edge (the dark weapon
//! model) hides that edge: the frame outside it is markedly darker than
//! inside, whatever the box does. A row whose left edge is a clear step
//! belongs to the box there. The left edge lies near the screen border,
//! which the weapon model does not reach. Elsewhere the box darkens
//! whatever lies behind its right edge, even dark scenes, so a step at the
//! left edge alone is gameplay (a wall passing that column). Above a band
//! that reaches above the closed chat, only the open panel can lie: it is
//! followed up by its right edge alone, as an overlay on the left of the
//! screen (the spectator's player card) can hide the left edge.
//!
//! The box leaves black black: rows whose frame next to an edge is too
//! dark to show a step neither start nor end a band, which runs on
//! through them. Where a filter lifts such black, the edges vanish into a
//! flat grey; the open chat is then still recognised from its input line,
//! which reaches below the closed box, and its white scrollbar thumb, and
//! its panel, whose place is fixed, is one band.

use crate::image::RgbView;

use super::{BOX_X, BoxBand, PANEL_X1, SEARCH_Y};

/// Width of each brightness strip next to a box edge.
const STRIP: i32 = 5;
/// Strips inside the box start this far from its edge; strips outside it
/// end this far from it (the edge itself is anti-aliased).
const EDGE_GAP: i32 = 2;
/// A row of the closed chat is inside the box where its edge ratio is
/// below this: the box fades out with its messages.
const MAX_RATIO: f32 = 0.95;
/// A clear edge step: a fully visible box darkens to about 0.66, the input
/// line of the open chat to 0.47, and filters that lift the shadows (gamma
/// 1.5, brightness +0.06 with contrast 0.85) to about 0.76. Above the
/// closed chat lies only the open panel, which does not fade: a run of
/// rows there needs [`MIN_ROWS`] rows below this, as textured gameplay
/// next to the edges darkens a few rows by 5 to 10 % by chance.
const STRONG_RATIO: f32 = 0.85;
/// The first row of the closed chat's box (port spec `CHAT_Y0`).
pub(super) const CLOSED_Y0: i32 = 930;
/// Added to both strip means so that black backgrounds give no ratio
/// spikes.
const RATIO_BIAS: f32 = 2.0;
/// The right edge is hidden where its ratio is at least this: outside
/// darker than inside, an object's own edge lies on the box edge (measured
/// 1.4 to 5 under the weapon model; an even background without the box
/// gives about 1, up to 1.15 on texture).
const HIDDEN_RATIO: f32 = 1.25;
/// The frame next to an edge is too dark to show the box below this grey
/// level: video compression crushes near-black to equal values on both
/// sides of the edge.
const DARK_LEVEL: f32 = 6.0;
/// Rows without a clear step the open panel's right edge may have between
/// clear rows (texture behind the edge).
const PANEL_GAP: i32 = 2;
/// The open panel's right edge has this ratio at most next to the white
/// scrollbar thumb (measured 0.14 to 0.4; a clear box edge is about 0.66).
const THUMB_RATIO: f32 = 0.5;
/// Rows of scrollbar thumb that show the chat is open (the thumb is at
/// least about 80 rows tall on recordings).
const THUMB_ROWS: usize = 20;
/// The first row of the open chat's panel.
pub(super) const PANEL_Y0: i32 = 787;
/// The first row of the open chat's input line, below the panel.
const INPUT_Y0: i32 = 1028;
/// Rows a band needs.
const MIN_ROWS: usize = 10;
/// A band that reaches above the closed chat and down to the input line is
/// the open panel; its top is fixed at [`PANEL_Y0`]. Where the band's top
/// was found up to this many rows lower (a darkening filter fades the
/// panel's top rows into the background; measured on a real open chat
/// under gamma 2.0: 12 rows), it starts there, so its first line counts.
const PANEL_SNAP: i32 = 16;
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

/// The edges of one row of the left panel.
#[derive(Debug, Clone, Copy)]
pub(super) struct RowEdge {
    /// The edge ratio ([`row_edge`]).
    ratio: f32,
    /// Whether the frame next to an edge is too dark to show the box: a
    /// multiplicative box leaves black black.
    dark: bool,
    /// Whether the open panel's right edge is a clear step.
    panel: bool,
    /// Whether the open chat's white scrollbar thumb lies right of the
    /// panel.
    thumb: bool,
}

/// The edges of one row: the edge ratio is the larger of the left and the
/// right edge ratio, or the left one alone where it is a clear step and
/// the right edge (the closed box's or the open panel's) is hidden.
///
/// The right edge is that of the closed box, or that of the open panel
/// where it is a clear step (the open panel does not fade, and a faint
/// step a few columns further in is just texture).
fn row_edge(row: &[[u8; 3]]) -> RowEdge {
    let ratio = |inside: f32, outside: f32| (inside + RATIO_BIAS) / (outside + RATIO_BIAS);
    let outside_left = strip_mean(row, BOX_X.0 - EDGE_GAP - STRIP);
    let outside_right = strip_mean(row, BOX_X.1 + EDGE_GAP + 1);
    let outside_panel = strip_mean(row, PANEL_X1 + EDGE_GAP + 1);
    let dark = outside_left < DARK_LEVEL || outside_right.max(outside_panel) < DARK_LEVEL;
    let left = ratio(strip_mean(row, BOX_X.0 + EDGE_GAP), outside_left);
    let right = ratio(strip_mean(row, BOX_X.1 - EDGE_GAP - STRIP), outside_right);
    let panel = ratio(strip_mean(row, PANEL_X1 - EDGE_GAP - STRIP), outside_panel);
    let ratio = if left <= STRONG_RATIO && right.max(panel) >= HIDDEN_RATIO {
        left
    } else if panel <= STRONG_RATIO {
        left.max(panel.min(right))
    } else {
        left.max(right)
    };
    RowEdge {
        ratio,
        dark,
        panel: panel <= STRONG_RATIO,
        thumb: panel <= THUMB_RATIO,
    }
}

/// Measures the box edges of one frame in `crop` (the left panel;
/// reference row `y` is crop row `y - origin_y`): replaces `edges` with
/// the edge ratio of every searched row ([`row_edge`], below
/// [`MAX_RATIO`] inside the box), `rows` with the edges of every searched
/// row, and `out` with the box bands.
pub(super) fn find_bands(
    crop: &RgbView<'_>,
    origin_y: i32,
    edges: &mut Vec<f32>,
    rows: &mut Vec<RowEdge>,
    out: &mut Vec<BoxBand>,
) {
    out.clear();
    edges.clear();
    rows.clear();
    for y in SEARCH_Y.0..SEARCH_Y.1 {
        let edge = row_edge(crop.row((y - origin_y) as u32).as_chunks::<3>().0);
        edges.push(edge.ratio);
        rows.push(edge);
    }
    let inside = |i: usize| rows[i].ratio < MAX_RATIO;
    // A run of rows inside the box may cross dark rows, which neither
    // start nor end it.
    let mut start = None;
    for i in 0..=rows.len() {
        let open = i < rows.len() && (inside(i) || rows[i].dark);
        match (open, start) {
            (true, None) if inside(i) => start = Some(i),
            (false, Some(s)) => {
                let end = (s..i).rev().find(|&k| inside(k)).map_or(s, |k| k + 1);
                push_run(rows, s, end, out);
                start = None;
            }
            _ => {}
        }
    }
    extend_panel(rows, out);
    open_panel(rows, out);
}

/// Adds the band of the run of rows `s..i` (indices into `rows`) to `out`
/// if it is long enough. Rows above the closed chat belong to it only
/// where it has at least [`MIN_ROWS`] clear rows there (hysteresis: the
/// open panel's rows over bright texture are fainter, textured gameplay
/// has no clear rows). Dark rows count towards neither.
fn push_run(rows: &[RowEdge], s: usize, i: usize, out: &mut Vec<BoxBand>) {
    let closed = (CLOSED_Y0 - SEARCH_Y.0) as usize;
    let clear = rows[s..i.min(closed).max(s)]
        .iter()
        .filter(|r| r.ratio < STRONG_RATIO)
        .count();
    let s = if s < closed && clear < MIN_ROWS {
        closed
    } else {
        s
    };
    let (sum, lit) = rows[s.min(i)..i]
        .iter()
        .filter(|r| r.ratio < MAX_RATIO)
        .fold((0.0, 0), |(sum, n), r| (sum + r.ratio, n + 1));
    if lit >= MIN_ROWS {
        push_merged(
            out,
            BoxBand {
                y0: SEARCH_Y.0 + s as i32,
                y1: SEARCH_Y.0 + i as i32,
                ratio: sum / lit as f32,
            },
        );
    }
}

/// Follows the open panel up from each band that reaches above the closed
/// chat (only the open panel does) by the panel's right edge alone, while
/// that is a clear step with at most [`PANEL_GAP`] rows between clear
/// rows: an overlay on the left of the screen (the spectator's player
/// card) can hide the left edge.
fn extend_panel(rows: &[RowEdge], out: &mut Vec<BoxBand>) {
    let closed = CLOSED_Y0 - SEARCH_Y.0;
    let bands = std::mem::take(out);
    for mut band in bands {
        let first = band.y0 - SEARCH_Y.0;
        if first < closed {
            let mut gap = 0;
            for k in (0..first).rev() {
                if rows[k as usize].panel {
                    band.y0 = SEARCH_Y.0 + k;
                    gap = 0;
                } else {
                    gap += 1;
                    if gap > PANEL_GAP {
                        break;
                    }
                }
            }
        }
        if band.y0 < CLOSED_Y0 && band.y1 == SEARCH_Y.1 && band.y0 - PANEL_Y0 <= PANEL_SNAP {
            band.y0 = band.y0.min(PANEL_Y0);
        }
        push_merged(out, band);
    }
}

/// Makes the whole open panel one band when the chat is open: the input
/// line reaches the last searched row (the closed box ends above it), or
/// its rows are black, and the scrollbar thumb is seen over at least
/// [`THUMB_ROWS`] rows. The panel's edges vanish where its background is
/// black, also where a filter lifts the shadows; its place is fixed.
fn open_panel(rows: &[RowEdge], out: &mut Vec<BoxBand>) {
    let input_rows = &rows[(INPUT_Y0 - SEARCH_Y.0) as usize..];
    let input =
        out.last().is_some_and(|band| band.y1 == SEARCH_Y.1) || input_rows.iter().all(|r| r.dark);
    if !input || !thumb_shows(rows) {
        return;
    }
    let ratio = out.iter().map(|b| b.ratio).fold(MAX_RATIO, f32::min);
    out.clear();
    out.push(BoxBand {
        y0: PANEL_Y0,
        y1: SEARCH_Y.1,
        ratio,
    });
}

/// Whether the open chat's scrollbar thumb shows in the searched rows
/// `rows` (as measured by [`find_bands`]): the thumb is white against any
/// background, so it shows where a filter hides the panel's edges.
pub(super) fn thumb_shows(rows: &[RowEdge]) -> bool {
    rows.iter().filter(|r| r.thumb).count() >= THUMB_ROWS
}

/// Appends `band`, merged with the previous band if the gap between them
/// is small or they overlap; a merged band keeps the lower (darker)
/// ratio.
fn push_merged(out: &mut Vec<BoxBand>, band: BoxBand) {
    match out.last_mut() {
        Some(last) if band.y0 - (last.y1 - 1) <= MERGE_GAP => {
            last.y0 = last.y0.min(band.y0);
            last.y1 = band.y1;
            last.ratio = last.ratio.min(band.ratio);
        }
        _ => out.push(band),
    }
}
