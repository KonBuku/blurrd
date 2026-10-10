//! The spectated player's name in the "switch player" widget.
//!
//! Measured on real recordings (reference pixels, left-anchored): while
//! the recording player is dead and spectates a teammate, the lower left
//! shows a thin light vertical bar at x 30 over rows 795..875 (80 rows,
//! luminance 130 to 250 on a backdrop of 9 to 21), the agent portrait
//! right of it, the teammate's name (16 px capitals, rows 828..843, from
//! x 121) and under it a mouse icon with the label "SWITCH PLAYER"
//! (x 122..271, rows 852..873).
//!
//! The structure is the bar: a thin straight vertical line (edges on
//! both of its sides, unlike a post or a wall corner) of exactly its
//! measured length (no edge just above or below it) at the gate's place,
//! and ink in the label's line under the name. Then the name slot gets
//! its box whenever it shows ink.

use blurrd_core::{NameKind, RectI, Region};

use super::edges;
use super::gate::{GATE_THRESHOLD, GateSpec};
use super::ink::{InkScratch, has_ink};
use super::sample::{Anchor, FrameView, RefPlane, ScreenMap};
use super::temporal::Slot;

/// The bar's column.
const BAR_X: i32 = 30;
/// The bar's rows.
const BAR_Y: (i32, i32) = (795, 875);
/// The name slot: every name length, with a margin of six rows.
const NAME_SLOT: RectI = RectI {
    x0: 116,
    y0: 821,
    x1: 420,
    y1: 851,
};
/// The label line under the name ("<", mouse icon, ">", "SWITCH
/// PLAYER").
const LABEL: RectI = RectI {
    x0: 120,
    y0: 852,
    x1: 274,
    y1: 874,
};
/// How far the bar may lie off its column (calibration error).
const SEARCH: i32 = 2;
/// How far the widget may lie off its rows (a slide, calibration error).
const SLIDE: i32 = 12;
/// Rows checked above and below the bar for its ends.
const END_ROWS: i32 = 10;
/// The bar must be an edge in at least this share of its rows (measured
/// on real widgets: 1.0).
const MIN_BAR: f32 = 0.7;
/// The bar must be a thin line (edges on both sides) in at least this
/// share of its rows (measured on real widgets: 1.0; a railing post or a
/// door frame, wider than a line, passed the edge test on real gameplay).
const MIN_LINE: f32 = 0.7;
/// Beyond its ends the bar's column may be an edge in at most this share
/// of the rows (a longer line is something else).
const MAX_BEYOND: f32 = 0.3;

/// The gate of the "switch player" widget: the bar.
pub const GATE: GateSpec = GateSpec {
    region: Region::Spectating,
    anchor: Anchor::Left,
    vertical: &[(BAR_X, BAR_Y.0 + 4, BAR_Y.1 - 4)],
    horizontal: &[],
    slide: 0,
    threshold: GATE_THRESHOLD,
};

/// The native zone the detector reads (reference pixels, left-anchored).
pub const ZONE: RectI = RectI {
    x0: BAR_X - SEARCH - 4,
    y0: BAR_Y.0 - SLIDE - END_ROWS - 2,
    x1: NAME_SLOT.x1 + 4,
    y1: BAR_Y.1 + SLIDE + END_ROWS + 2,
};

/// Reads the name slot of the "switch player" widget.
#[derive(Debug, Clone, Default)]
pub struct SpectatingReader {
    plane: RefPlane,
    ink: InkScratch,
}

impl SpectatingReader {
    /// A reader.
    pub fn new() -> Self {
        Self::default()
    }

    /// The name slot in `view` (a native crop containing [`ZONE`]), or
    /// `None` if the widget's bar and label are not found.
    pub fn read(&mut self, view: &FrameView<'_>, map: &ScreenMap) -> Option<Vec<Slot>> {
        self.plane.sample(view, map, Anchor::Left, ZONE, 1);
        // The bar's vertical position (a widget sliding in): where its
        // rows are an edge and the rows just beyond its ends are not. The
        // edge alone is as strong a few rows off (the bar's end rows
        // blur into their neighbours), the ends tell the place.
        let (rank, bar, x, dy) = (-SLIDE..=SLIDE)
            .map(|dy| {
                let (top, bottom) = (BAR_Y.0 + dy, BAR_Y.1 + dy);
                let (bar, dx) = edges::vertical(&self.plane, BAR_X, top, bottom, SEARCH);
                let x = BAR_X + dx;
                let above = edges::vertical(&self.plane, x, top - END_ROWS, top - 1, 0).0;
                let below = edges::vertical(&self.plane, x, bottom + 1, bottom + END_ROWS, 0).0;
                (bar - above.max(below), bar, x, dy)
            })
            .fold(
                (f32::MIN, 0.0, 0, 0),
                |best, c| if c.0 > best.0 { c } else { best },
            );
        // The rows beyond the ends may be an edge in at most
        // `MAX_BEYOND` of the rows (a longer line is something else).
        if bar < MIN_BAR || bar - rank > MAX_BEYOND {
            return None;
        }
        let (top, bottom) = (BAR_Y.0 + dy, BAR_Y.1 + dy);
        let (column, row0) = self.plane.cell(x, top);
        let (_, row1) = self.plane.cell(x, bottom);
        if edges::line_share(&self.plane, column, row0, row1) < MIN_LINE {
            return None;
        }
        let shift = |r: RectI| RectI {
            y0: r.y0 + dy,
            y1: r.y1 + dy,
            ..r
        };
        if !has_ink(&self.plane, shift(LABEL), &mut self.ink) {
            return None;
        }
        let rect = shift(NAME_SLOT);
        let inner = RectI {
            x0: rect.x0 + 2,
            y0: rect.y0 + 2,
            x1: rect.x1 - 2,
            y1: rect.y1 - 2,
        };
        Some(vec![Slot {
            kind: NameKind::Spectated,
            rect,
            ink: has_ink(&self.plane, inner, &mut self.ink),
        }])
    }
}
