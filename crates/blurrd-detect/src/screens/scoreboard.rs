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
/// Where the mean of the structure lines stays below [`MIN_LINES`], the
/// board is still found if the header band's lower edge (the panel's
/// strongest line) scores at least this, at least [`MIN_FAINT_COUNT`] of
/// the other lines score [`MIN_FAINT`], and the panel's left edge holds
/// ([`MIN_LEFT_EDGE`]). In downscaled recordings the 1 px row separators
/// lose half their contrast and fall below the edge step, and the header
/// band's upper edge lies over the scene (measured on a real board at
/// 720p: header lower edge 0.96 to 1.0, upper edge 0.16 to 0.79, row
/// boundaries 0.04 to 0.99, mean 0.46 to 0.49, left edge 0.67 to 0.89).
const MIN_HEADER: f32 = 0.8;
/// See [`MIN_HEADER`].
const MIN_FAINT: f32 = 0.3;
/// See [`MIN_HEADER`].
const MIN_FAINT_COUNT: usize = 1;
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
/// Where the structure lines over the name columns are this strong, the
/// lines across the rest of the panel need only [`MIN_WIDE_WEAK`]: in a
/// greyscale copy downscaled to 720p the 1 px separators there fade too
/// (measured on a real board: name-column lines 0.85 to 0.89, wide lines
/// 0.37 to 0.51, left edge 0.28 to 0.31; 0.83 wide at 1080p).
const MIN_STRONG_LINES: f32 = 0.8;
/// See [`MIN_STRONG_LINES`].
const MIN_WIDE_WEAK: f32 = 0.35;
/// The weak wide lines also confirm the board where the header band's
/// edges and the first row boundary each score [`MIN_HEADER`] and the
/// panel's left edge at least this (measured on two more real boards in
/// greyscale at 720p: name-column lines 0.68 to 0.74, header lines 0.81
/// to 1.0, left edge 0.48 to 0.57, wide lines 0.44 to 0.51).
const MIN_LEFT_WEAK: f32 = 0.45;
/// The one-block (deathmatch) layout needs at least this score of its own
/// row boundaries after the first five rows ...
const MIN_BLOCK: f32 = 0.5;
/// ... and must beat the 5v5 layout's boundaries by this margin. On real
/// 5v5 boards the block's boundaries score 0.20 to 0.35 (the edges of the
/// round-history strip and its icons); at 720p the enemy rows' 1 px
/// separators fade and the 5v5 boundaries fell to 0.26 to 0.28 in single
/// frames, so a plain comparison picked the block layout. The 5v5 layout
/// is the verified one and wins every close call.
const BLOCK_MARGIN: f32 = 0.2;
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
    outlines: &[],
    outline_span: (0, 0),
    outline_rows: 0,
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
        let rows = (FIRST_ROW + dy + 2, FIRST_ROW + PITCH * TEAM_ROWS + dy - 2);
        let left_edge = edges::vertical(&self.plane, PANEL_X0, rows.0, rows.1, 1).0;
        if score < MIN_LINES && !(left_edge >= MIN_LEFT_EDGE && self.header_holds(dy)) {
            return None;
        }
        if left_edge < MIN_LEFT_EDGE {
            let wide = self.wide_lines(view, map, dy);
            let agree =
                score >= MIN_STRONG_LINES || (left_edge >= MIN_LEFT_WEAK && self.header_strong(dy));
            let needed = if agree { MIN_WIDE_WEAK } else { MIN_WIDE_LINES };
            if wide < needed {
                return None;
            }
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
        let tops = if is_block(boundaries(&teams), boundaries(&block)) {
            block
        } else {
            teams
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

    /// Whether the header band's lower edge and enough of the other
    /// structure lines hold at the offset `dy` ([`MIN_HEADER`]; each line
    /// may lie one row off it).
    fn header_holds(&self, dy: i32) -> bool {
        let score = |&(y, x0, x1): &(i32, i32, i32)| {
            edges::horizontal_set(&self.plane, &[(y + dy, x0, x1)], 0).0
        };
        let header = LINES.iter().position(|&(y, _, _)| y == FIRST_ROW);
        let faint = LINES
            .iter()
            .enumerate()
            .filter(|&(i, line)| Some(i) != header && score(line) >= MIN_FAINT)
            .count();
        header.is_some_and(|i| score(&LINES[i]) >= MIN_HEADER) && faint >= MIN_FAINT_COUNT
    }

    /// Whether the header band's two edges and the first row boundary each
    /// score at least [`MIN_HEADER`] at the offset `dy` (each may lie one
    /// row off it).
    fn header_strong(&self, dy: i32) -> bool {
        LINES[..3].iter().all(|&(y, x0, x1)| {
            edges::horizontal_set(&self.plane, &[(y + dy, x0, x1)], 0).0 >= MIN_HEADER
        })
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

/// Whether the row boundaries after the first five rows show the
/// one-block layout: `teams` and `block` are the scores of the two
/// layouts' boundaries.
fn is_block(teams: f32, block: f32) -> bool {
    block >= MIN_BLOCK && block >= teams + BLOCK_MARGIN
}

#[cfg(test)]
mod tests {
    use super::{
        FIRST_ROW, HEADER_TOP, PANEL_X0, PANEL_X1, PITCH, ScoreboardReader, TEAM_ROWS, is_block,
    };
    use blurrd_testkit::hud::killfeed::Team;
    use blurrd_testkit::hud::scoreboard::{ScoreRow, ScoreboardMode, ScoreboardSpec};
    use blurrd_testkit::hud::screen::ScreenTiming;
    use blurrd_testkit::{Background, FontSet, Layout, NameStyle, Palette, Rng, Scene, synth_name};

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

    #[test]
    fn the_header_and_the_left_edge_confirm_a_board_with_faded_separators() {
        // Downscaled: the separators below the first row boundary fall
        // under the edge step, and the header band's top lies on a scene
        // of its own tone. The header's lower edge, the first boundary and
        // the panel's left edge still confirm the board.
        let (w, h) = (1920usize, 1080usize);
        let mut data = vec![140u8; 3 * w * h];
        let fill = |data: &mut [u8], y0: i32, y1: i32, x0: i32, x1: i32, v: u8| {
            for y in y0..y1 {
                let row = 3 * y as usize * w;
                data[row + 3 * x0 as usize..row + 3 * x1 as usize].fill(v);
            }
        };
        let rows = FIRST_ROW + PITCH * TEAM_ROWS;
        fill(&mut data, FIRST_ROW, rows, PANEL_X0, PANEL_X1, 120);
        fill(&mut data, HEADER_TOP - 30, FIRST_ROW, 0, w as i32, 60);
        let y = FIRST_ROW + PITCH;
        fill(&mut data, y, y + 1, PANEL_X0, PANEL_X1, 200);
        for k in 2..TEAM_ROWS {
            let y = FIRST_ROW + PITCH * k;
            fill(&mut data, y, y + 1, PANEL_X0, PANEL_X1, 126);
        }
        assert!(read(&data));
        // Without the panel's left edge, these lines are no board.
        fill(&mut data, FIRST_ROW, rows, 0, PANEL_X0, 120);
        assert!(!read(&data));
    }

    #[test]
    fn close_calls_keep_the_five_versus_five_layout() {
        // Measured on a real 5v5 board at 720p.
        assert!(!is_block(0.26, 0.29));
        assert!(!is_block(0.9, 0.3));
        // A one-block board shows its own boundaries only.
        assert!(is_block(0.3, 0.9));
        assert!(!is_block(0.1, 0.4));
    }

    /// The number of name slots read from a synthetic board in `mode` at
    /// 1280 x 720.
    fn slots_at_720p(mode: ScoreboardMode, rows: usize) -> usize {
        let mut rng = Rng::new(5);
        let rows = (0..rows)
            .map(|i| ScoreRow {
                name: synth_name(&mut rng, NameStyle::Latin(10)),
                team: if i < 5 { Team::Ally } else { Team::Enemy },
                own: i == 0,
                dead: i == 3,
            })
            .collect();
        let layout = Layout::wide(1280, 720);
        let scene = Scene::new(layout, Background::noise(5), Palette::DEFAULT, 2).with_scoreboard(
            &FontSet::latin(),
            ScoreboardSpec {
                rows,
                mode,
                timing: ScreenTiming::instant(0, 4),
            },
        );
        let frame = scene.render(1).frame;
        let view = FrameView::native(
            RgbView::new(
                frame.data(),
                frame.width(),
                frame.height(),
                3 * frame.width() as usize,
            )
            .unwrap(),
        );
        let map = ScreenMap::from(&layout.geometry());
        ScoreboardReader::new()
            .read(&view, &map, &GateSignal::default())
            .map_or(0, |slots| slots.len())
    }

    #[test]
    fn picks_the_layout_at_720p() {
        assert_eq!(slots_at_720p(ScoreboardMode::Teams, 10), 10);
        assert_eq!(slots_at_720p(ScoreboardMode::Deathmatch, 14), 14);
    }
}
