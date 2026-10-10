//! Player names in the scoreboard (tab) name column.
//!
//! Measured on real recordings (reference pixels, centre-anchored): the
//! panel spans x 572..1348; a dark header band at y 309..338 holds the
//! column titles; player rows follow on a 34 px pitch. In a 5v5 game the
//! allied rows start at y 338 and the enemy rows at y 569, with the
//! round-history strip in between; a deathmatch board (not verified, no
//! recording) is drawn as one block from y 338. The name cell of a row
//! spans x 636..838 (names start at x 644, 10 px capitals 12 to 22 px
//! below the row top, descenders to + 25); the agent icon sits at
//! x 573..609. The scoreboard opens and closes within one frame.
//!
//! The panel is found from structure only: its left and right edges, the
//! edges of the header band and the boundaries of the first five rows
//! (the gate), then the horizontal lines once more at native scale to
//! place the row grid. The panel's left edge confirms the board; where it
//! has the same luminance as the scene behind it (a greyscale or
//! colour-blind filter can remove the only step there, a hue change), the
//! same lines across the rest of the panel's width confirm it instead.
//! The layout (teams or one block) is the grid whose row boundaries show
//! more edges. Every row of the grid whose name cell shows ink gets its
//! slot box, so the dim names of dead players and names in any script
//! are covered without reading them.

use blurrd_core::{NameKind, RectI, Region};

use super::edges;
use super::gate::{GATE_STEP, GATE_THRESHOLD, GateSignal, GateSpec};
use super::ink::{InkScratch, has_ink};
use super::sample::{Anchor, FrameView, RefPlane, ScreenMap};
use super::temporal::Slot;

/// The panel's left edge.
const PANEL_X0: i32 = 572;
/// The panel's right edge.
const PANEL_X1: i32 = 1348;
/// Top of the header band.
const HEADER_TOP: i32 = 309;
/// Top of the first row (bottom of the header band).
const FIRST_ROW: i32 = 338;
/// Distance between row tops.
const PITCH: i32 = 34;
/// Top of the first enemy row in a 5v5 board.
const ENEMY_ROW: i32 = 569;
/// Rows per team in a 5v5 board.
const TEAM_ROWS: i32 = 5;
/// Rows of a deathmatch board.
const BLOCK_ROWS: i32 = 14;
/// The name cell (x range); its rows are the row band.
const CELL_X: (i32, i32) = (636, 838);
/// The slot box rows below the row top.
const CELL_Y: (i32, i32) = (2, 32);
/// Where the ink test looks, relative to the row top.
const INK_Y: (i32, i32) = (6, 30);
/// How far the panel may be off its place (calibration error, slide);
/// less than half the row pitch, so that the line set shifted by one row
/// cannot match the panel.
const SLIDE: i32 = 16;
/// How far the native lines may lie from the gate's offset: one gate
/// cell plus a margin.
const REFINE: i32 = GATE_STEP + 2;
/// The structure lines must show at least this mean score at native
/// scale (measured on real boards: 0.81).
const MIN_LINES: f32 = 0.5;
/// The panel's left edge beside the first five rows must score at least
/// this at native scale (measured on real boards: 1.0).
const MIN_LEFT_EDGE: f32 = 0.6;
/// The lines across the rest of the panel (right of the name cell) must
/// show at least this mean score when the left edge is not found
/// (measured on a real board in greyscale: 0.83, with a left edge of
/// 0.30).
const MIN_WIDE_LINES: f32 = 0.6;
/// The x range of those lines.
const WIDE_X: (i32, i32) = (CELL_X.1 + 4, PANEL_X1 - 8);
/// The structure lines every board has: the header band's edges and the
/// boundaries of the first five rows, `(y, x0, x1)` inside the native
/// zone. Measured edge scores on a real board: 0.73, 1.0, 0.72, 0.67,
/// 0.74, 0.99.
const LINES: [(i32, i32, i32); 6] = [
    (HEADER_TOP, 584, 836),
    (FIRST_ROW, 584, 836),
    (FIRST_ROW + PITCH, 584, 836),
    (FIRST_ROW + 2 * PITCH, 584, 836),
    (FIRST_ROW + 3 * PITCH, 584, 836),
    (FIRST_ROW + 4 * PITCH, 584, 836),
];
/// Right end of the gate's lines: across the icon, name, ultimate and KDA
/// columns (the row bands and separators span the whole panel).
const GATE_LINES_X1: i32 = 1000;
/// The same lines over more of the panel's width, for the gate.
const GATE_LINES: [(i32, i32, i32); 6] = [
    (HEADER_TOP, 584, GATE_LINES_X1),
    (FIRST_ROW, 584, GATE_LINES_X1),
    (FIRST_ROW + PITCH, 584, GATE_LINES_X1),
    (FIRST_ROW + 2 * PITCH, 584, GATE_LINES_X1),
    (FIRST_ROW + 3 * PITCH, 584, GATE_LINES_X1),
    (FIRST_ROW + 4 * PITCH, 584, GATE_LINES_X1),
];

/// The gate of the scoreboard.
pub const GATE: GateSpec = GateSpec {
    region: Region::Scoreboard,
    anchor: Anchor::Center,
    vertical: &[(PANEL_X0, 344, 500), (PANEL_X1, 344, 500)],
    horizontal: &GATE_LINES,
    slide: SLIDE,
    threshold: GATE_THRESHOLD,
};

/// The part of the zone read on every candidate frame (reference pixels,
/// centre-anchored): the icon and name columns of every row position.
const NAMES: RectI = RectI {
    x0: PANEL_X0 - 4,
    y0: HEADER_TOP - SLIDE - REFINE - 4,
    x1: CELL_X.1 + 4,
    y1: FIRST_ROW + PITCH * BLOCK_ROWS + SLIDE + REFINE + 4,
};

/// Rows of reference margin around the structure lines in the part right
/// of the name cell: one row of tolerance and the cells the edge test
/// looks at beyond a line.
const WIDE_MARGIN: i32 = 4;

/// The native zone the detector reads (reference pixels,
/// centre-anchored): the icon and name columns of every row position and
/// the rest of the panel's width, read only when the left edge is not
/// found.
pub const ZONE: RectI = RectI {
    x1: PANEL_X1 + 4,
    ..NAMES
};

/// Reads the name slots of the scoreboard.
#[derive(Debug, Clone, Default)]
pub struct ScoreboardReader {
    plane: RefPlane,
    wide: RefPlane,
    ink: InkScratch,
}

impl ScoreboardReader {
    /// A reader.
    pub fn new() -> Self {
        Self::default()
    }

    /// The name slots in `view` (a native crop containing [`ZONE`]), or
    /// `None` if the board's structure lines are not found. `gate` is the
    /// frame's gate signal: its offset places the search for the lines.
    pub fn read(
        &mut self,
        view: &FrameView<'_>,
        map: &ScreenMap,
        gate: &GateSignal,
    ) -> Option<Vec<Slot>> {
        self.plane.sample(view, map, Anchor::Center, NAMES, 1);
        let hint = gate.offset.clamp(-SLIDE, SLIDE);
        let lines = LINES.map(|(y, x0, x1)| (y + hint, x0, x1));
        let (score, refine) = edges::horizontal_set(&self.plane, &lines, REFINE);
        let dy = hint + refine;
        if score < MIN_LINES {
            return None;
        }
        let rows = (FIRST_ROW + dy + 2, FIRST_ROW + PITCH * TEAM_ROWS + dy - 2);
        if edges::vertical(&self.plane, PANEL_X0, rows.0, rows.1, 1).0 < MIN_LEFT_EDGE
            && self.wide_lines(view, map, dy) < MIN_WIDE_LINES
        {
            return None;
        }
        let teams: Vec<i32> = (0..TEAM_ROWS)
            .map(|k| FIRST_ROW + PITCH * k)
            .chain((0..TEAM_ROWS).map(|k| ENEMY_ROW + PITCH * k))
            .collect();
        let block: Vec<i32> = (0..BLOCK_ROWS).map(|k| FIRST_ROW + PITCH * k).collect();
        // The two layouts share the first five rows; they differ in the
        // row boundaries after them.
        let boundaries = |tops: &[i32]| {
            let lines: Vec<(i32, i32, i32)> = tops[TEAM_ROWS as usize..]
                .iter()
                .take(TEAM_ROWS as usize)
                .map(|t| (t + dy, CELL_X.0, CELL_X.1))
                .collect();
            edges::horizontal_set(&self.plane, &lines, 0).0
        };
        let tops = if boundaries(&teams) >= boundaries(&block) {
            teams
        } else {
            block
        };
        let slots = tops
            .iter()
            .map(|&top| {
                let top = top + dy;
                let ink = RectI {
                    x0: CELL_X.0 + 4,
                    y0: top + INK_Y.0,
                    x1: CELL_X.1 - 2,
                    y1: top + INK_Y.1,
                };
                Slot {
                    kind: NameKind::ScoreboardRow,
                    rect: RectI {
                        x0: CELL_X.0,
                        y0: top + CELL_Y.0,
                        x1: CELL_X.1,
                        y1: top + CELL_Y.1,
                    },
                    ink: has_ink(&self.plane, ink, &mut self.ink),
                }
            })
            .collect();
        Some(slots)
    }

    /// The mean score of the structure lines right of the name cell, at
    /// the offset `dy` the name columns gave (each line may lie one row
    /// off it).
    fn wide_lines(&mut self, view: &FrameView<'_>, map: &ScreenMap, dy: i32) -> f32 {
        let (top, _, _) = LINES[0];
        let (bottom, _, _) = LINES[LINES.len() - 1];
        let rect = RectI {
            x0: WIDE_X.0,
            y0: top + dy - WIDE_MARGIN,
            x1: WIDE_X.1,
            y1: bottom + dy + WIDE_MARGIN,
        };
        self.wide.sample(view, map, Anchor::Center, rect, 1);
        let lines = LINES.map(|(y, _, _)| (y + dy, WIDE_X.0, WIDE_X.1));
        edges::horizontal_set(&self.wide, &lines, 0).0
    }
}

#[cfg(test)]
mod tests {
    use super::{FIRST_ROW, HEADER_TOP, PANEL_X0, PANEL_X1, PITCH, ScoreboardReader, TEAM_ROWS};
    use crate::image::RgbView;
    use crate::screens::gate::GateSignal;
    use crate::screens::sample::{FrameView, ScreenMap};

    /// A reference frame in flat grey with the board's header band and
    /// row separators but no step at the panel's left edge (the panel has
    /// the luminance of the scene behind it), the lines drawn from the
    /// left edge to `x1`.
    fn board(x1: i32) -> Vec<u8> {
        let (w, h) = (1920usize, 1080usize);
        let mut data = vec![140u8; 3 * w * h];
        let mut fill = |y0: i32, y1: i32, v: u8| {
            for y in y0..y1 {
                let row = 3 * y as usize * w;
                data[row + 3 * PANEL_X0 as usize..row + 3 * x1 as usize].fill(v);
            }
        };
        fill(HEADER_TOP, FIRST_ROW, 60);
        for k in 1..TEAM_ROWS {
            let y = FIRST_ROW + PITCH * k;
            fill(y, y + 1, 200);
        }
        data
    }

    fn read(data: &[u8]) -> bool {
        let view = FrameView::native(RgbView::new(data, 1920, 1080, 3 * 1920).unwrap());
        ScoreboardReader::new()
            .read(&view, &ScreenMap::REFERENCE, &GateSignal::default())
            .is_some()
    }

    #[test]
    fn lines_across_the_panel_confirm_a_board_without_a_left_edge() {
        assert!(read(&board(PANEL_X1)));
        // The same lines only over the name columns are not a board.
        assert!(!read(&board(super::CELL_X.1 + 4)));
    }
}
