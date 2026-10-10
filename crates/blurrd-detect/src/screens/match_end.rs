//! Names on the match-end and MVP screens.
//!
//! Not verified: the local test media contain no match-end or MVP screen
//! (decision D5); their round-end banners show a result word and a
//! reason line but no player name, and must not open this gate. The
//! geometry follows the synthetic stand-in of the test kit
//! (centre-anchored): one card per player, 220 px wide on a 236 px pitch
//! from x 372, at y 400..760, with a name strip at y 712..746. The
//! structure is the cards' vertical edges and their horizontal edges (top,
//! strip top and bottom, bottom); every card whose own rectangle is found
//! (both side edges and both strip edges) and whose strip shows ink gets
//! its strip as a slot box.

use blurrd_core::{NameKind, RectI, Region};

use super::edges;
use super::gate::{GATE_STEP, GATE_THRESHOLD, GateSignal, GateSpec};
use super::ink::{InkScratch, has_ink};
use super::sample::{Anchor, FrameView, RefPlane, ScreenMap};
use super::temporal::Slot;

/// Left edge of the first card.
const CARD_X0: i32 = 372;
/// Width of a card.
const CARD_W: i32 = 220;
/// Distance between card left edges.
const CARD_PITCH: i32 = 236;
/// Number of card positions.
const CARDS: i32 = 5;
/// Top and bottom of the cards.
const CARD_Y: (i32, i32) = (400, 760);
/// The name strip of a card (y range).
const STRIP_Y: (i32, i32) = (712, 746);
/// How far the cards may be off their place (less than the distance
/// between the strip's edges).
const SLIDE: i32 = 20;
/// How far the native lines may lie from the gate's offset.
const REFINE: i32 = GATE_STEP + 2;
/// Reference pixels per cell of the native plane.
const STEP: i32 = 2;
/// The cards' horizontal edges must show at least this mean score.
const MIN_LINES: f32 = 0.5;
/// A card exists where its left and right edges and the top and bottom
/// edges of its name strip each score at least this (a long line of
/// floor tiles matched the left edges and the lines across all cards
/// on real gameplay).
const MIN_CARD_EDGE: f32 = 0.5;
/// Right edge of the last card position.
const CARDS_X1: i32 = CARD_X0 + CARD_PITCH * (CARDS - 1) + CARD_W;
/// The cards' top edge `(y, x0, x1)` across all positions.
const TOP_LINE: (i32, i32, i32) = (CARD_Y.0, CARD_X0, CARDS_X1);
/// The cards' strip and bottom edges `(y, x0, x1)` across all positions.
const LOWER_LINES: [(i32, i32, i32); 3] = [
    (STRIP_Y.0, CARD_X0, CARDS_X1),
    (STRIP_Y.1, CARD_X0, CARDS_X1),
    (CARD_Y.1, CARD_X0, CARDS_X1),
];
/// The horizontal edges of the first card only, for the gate (every
/// match-end screen shows at least one card).
const GATE_LINES: [(i32, i32, i32); 4] = [
    (CARD_Y.0, CARD_X0, CARD_X0 + CARD_W),
    (STRIP_Y.0, CARD_X0, CARD_X0 + CARD_W),
    (STRIP_Y.1, CARD_X0, CARD_X0 + CARD_W),
    (CARD_Y.1, CARD_X0, CARD_X0 + CARD_W),
];
/// Rows the vertical card edges are read over.
const EDGE_Y: (i32, i32) = (CARD_Y.0 + 20, STRIP_Y.0 - 12);

/// The gate of the match-end screens: the card edges.
pub const GATE: GateSpec = GateSpec {
    region: Region::MatchEnd,
    anchor: Anchor::Center,
    vertical: &[
        (CARD_X0, EDGE_Y.0, EDGE_Y.1),
        (CARD_X0 + CARD_W, EDGE_Y.0, EDGE_Y.1),
        (CARD_X0 + 2 * CARD_PITCH, EDGE_Y.0, EDGE_Y.1),
        (CARD_X0 + 2 * CARD_PITCH + CARD_W, EDGE_Y.0, EDGE_Y.1),
        (CARD_X0 + 4 * CARD_PITCH, EDGE_Y.0, EDGE_Y.1),
        (CARDS_X1, EDGE_Y.0, EDGE_Y.1),
    ],
    outlines: &[],
    outline_span: (0, 0),
    outline_rows: 0,
    horizontal: &GATE_LINES,
    slide: SLIDE,
    threshold: GATE_THRESHOLD,
};

/// The native zone the detector reads (reference pixels,
/// centre-anchored).
pub const ZONE: RectI = RectI {
    x0: CARD_X0 - 8,
    y0: CARD_Y.0 - SLIDE - REFINE - 4,
    x1: CARDS_X1 + 8,
    y1: CARD_Y.1 + SLIDE + REFINE + 4,
};

/// Rows of reference margin around a line band (placed at the gate's
/// offset): the native refinement, one cell of tolerance and the cells
/// the edge test looks at beyond the line.
const BAND: i32 = REFINE + 4 * STEP;
/// Columns of reference margin around a side edge: the search cell and
/// the cells the edge test looks at.
const SIDE: i32 = 4 * STEP;

/// Reads the name slots of the match-end screens.
///
/// The cards are large and their art does not matter, so only three
/// narrow parts of the zone are resampled: the band of the cards' top
/// edge, the band of the strip and bottom edges, and a column around
/// each side edge.
#[derive(Debug, Clone, Default)]
pub struct MatchEndReader {
    top: RefPlane,
    strip: RefPlane,
    side: RefPlane,
    ink: InkScratch,
}

impl MatchEndReader {
    /// A reader.
    pub fn new() -> Self {
        Self::default()
    }

    /// The name slots in `view` (a native crop containing [`ZONE`]) of the
    /// cards that are there, or `None` if no card is found. `gate` places
    /// the search.
    pub fn read(
        &mut self,
        view: &FrameView<'_>,
        map: &ScreenMap,
        gate: &GateSignal,
    ) -> Option<Vec<Slot>> {
        let hint = gate.offset.clamp(-SLIDE, SLIDE);
        let band = |y0: i32, y1: i32| RectI {
            x0: ZONE.x0,
            y0: y0 + hint - BAND,
            x1: ZONE.x1,
            y1: y1 + hint + BAND,
        };
        // A 2 px grid keeps the edges and the strip text and halves the
        // cost in each direction.
        self.top
            .sample(view, map, Anchor::Center, band(CARD_Y.0, CARD_Y.0), STEP);
        self.strip
            .sample(view, map, Anchor::Center, band(STRIP_Y.0, CARD_Y.1), STEP);
        // The strip and bottom edges place the cards; the top edge must
        // be there at the same offset (one cell either way).
        let lower = LOWER_LINES.map(|(y, x0, x1)| (y + hint, x0, x1));
        let (score, refine) = edges::horizontal_set(&self.strip, &lower, REFINE / STEP);
        let dy = hint + refine;
        let (y, x0, x1) = TOP_LINE;
        let (top, _) = edges::horizontal_set(&self.top, &[(y + dy, x0, x1)], 0);
        // The mean over all four edges, as one set.
        if (3.0 * score + top) / 4.0 < MIN_LINES {
            return None;
        }
        let slots: Vec<Slot> = (0..CARDS)
            .filter_map(|k| {
                let x0 = CARD_X0 + CARD_PITCH * k;
                let x1 = x0 + CARD_W;
                let sides = [x0, x1].map(|x| {
                    let rect = RectI {
                        x0: x - SIDE,
                        y0: EDGE_Y.0 + dy,
                        x1: x + SIDE,
                        y1: EDGE_Y.1 + dy,
                    };
                    self.side.sample(view, map, Anchor::Center, rect, STEP);
                    edges::vertical(&self.side, x, EDGE_Y.0 + dy, EDGE_Y.1 + dy, 1).0
                });
                let strip = |y| edges::horizontal_set(&self.strip, &[(y + dy, x0, x1)], 1).0;
                let edge = sides[0]
                    .min(sides[1])
                    .min(strip(STRIP_Y.0))
                    .min(strip(STRIP_Y.1));
                let rect = RectI {
                    x0: x0 + 2,
                    y0: STRIP_Y.0 + dy,
                    x1: x0 + CARD_W - 2,
                    y1: STRIP_Y.1 + dy,
                };
                let inner = RectI {
                    x0: rect.x0 + 2,
                    y0: rect.y0 + 2,
                    x1: rect.x1 - 2,
                    y1: rect.y1 - 2,
                };
                (edge >= MIN_CARD_EDGE).then(|| Slot {
                    kind: NameKind::MatchEndName,
                    rect,
                    ink: has_ink(&self.strip, inner, &mut self.ink),
                })
            })
            .collect();
        // At least one card must be there.
        (!slots.is_empty()).then_some(slots)
    }
}
