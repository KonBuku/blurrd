//! Names in the death recap: the "killed by" panel and the combat report.
//!
//! Measured on real recordings (reference pixels, right-anchored): the
//! panel spans x 1525..1900 and is centred vertically on y 540; its
//! height grows with the number of combat-report rows (one row: y
//! 389..691; four rows and two timeline rows: y 284..800). From the top:
//! the "KILLED BY" header, the killer's card art, a strip with the
//! killer's name on the left, the "COMBAT REPORT" header, then one 58 px
//! row per opponent with an outlined card at x 1638..1788 whose lower
//! half holds the opponent's name. The strip ends 40 to 45 px above the
//! first card.
//!
//! The combat report also opens without the "killed by" part while the
//! player is alive (with the scoreboard), at the same x positions.
//!
//! Because the panel moves with its content, the reader finds the cards
//! from structure: their left and right outlines are long, straight
//! vertical edges. Runs of outline rows that marks between the cards
//! joined are split on the 58 px row pitch; cards that are fainter (a
//! fade, a textured portrait touching the outline) are filled in on the
//! pitch from the cards found, with a relaxed outline test. Each card's
//! lower half is a name slot (with a small margin). The killer's strip is
//! found as a band with two straight horizontal edges left of the cards,
//! above the first card; its rows are the killer's name slot.

use blurrd_core::{NameKind, RectI, Region};

use super::edges::{self, LINE_W, edge_at, line_at};
use super::gate::{GATE_THRESHOLD, GateSpec};
use super::ink::{InkScratch, has_ink};
use super::sample::{Anchor, FrameView, RefPlane, ScreenMap};
use super::temporal::Slot;

/// The panel's left edge.
const PANEL_X0: i32 = 1525;
/// The panel's right edge.
const PANEL_X1: i32 = 1900;
/// The vertical centre of the panel.
const PANEL_CY: i32 = 540;
/// Highest and lowest reference rows the panel can reach.
const PANEL_Y: (i32, i32) = (140, 940);
/// The card outlines of the combat-report rows: the left edge lies
/// between x 1637 and 1638, the right edge between 1787 and 1788.
const CARD_X: (i32, i32) = (1638, 1788);
/// Shortest and longest card (measured: 58 px).
const CARD_H: (i32, i32) = (40, 70);
/// The part of a card right of the portrait (x range).
const INSIDE_X: (i32, i32) = (1704, 1782);
/// Largest share of neighbouring cells inside a card that may differ
/// clearly (measured on real cards: 0.08 to 0.14; textured card art
/// about 0.6).
const MAX_INSIDE_EDGES: f32 = 0.3;
/// How far a card's top and bottom outlines may lie from the ends of its
/// run of outline rows (marks next to the card extend the run).
const LINE_SEARCH: i32 = 4;
/// Rows without an outline edge a card may contain (the gap between two
/// cards is 8 rows).
const CARD_GAP: i32 = 5;
/// Rows a card's name slot extends above the card's middle and below its
/// bottom (into the 8-row gap to the next card): a faint card in a fade
/// is found a few rows off its place.
const CARD_MARGIN: (i32, i32) = (2, 6);
/// Distance between the tops of two cards (measured: 58 px).
const CARD_PITCH: i32 = 58;
/// Height of a card in a pitch-filled position (measured: 50 px).
const FILL_H: i32 = 50;
/// Largest difference, in rows, between a joined run and a whole number
/// of cards on the pitch. Marks next to the first or last card extend
/// the run (measured: 5 rows on a real recap, 7 under a vibrance
/// filter); each card's own outlines are checked afterwards.
const SPLIT_TOLERANCE: i32 = 12;
/// A pitch-filled card's left and right outlines must each be a line in
/// at least this share of its rows.
const MIN_FILL: f32 = 0.5;
/// A card's top and bottom outlines must each score at least this (the
/// agent portrait can touch them; measured on real cards: 0.96 to 1.0).
const MIN_LINE: f32 = 0.5;
/// A card's right outline, its top and bottom outlines and the panel's
/// left edge beside it must each score at least this (measured on real
/// cards: 1.0, 0.96 to 1.0, 0.99), so that a wall or the weapon list
/// next to the panel position is never taken for a card.
const MIN_OUTLINE: f32 = 0.6;
/// Without the panel's left edge beside a card, its right, top and bottom
/// outlines must each score at least this.
const MIN_STRONG: f32 = 0.8;
/// The killer's name slot in the strip (x range).
const STRIP_X: (i32, i32) = (1527, 1712);
/// Where the strip's band edges are read (x range): the strip spans the
/// whole panel, under the card art. The combat report shown without the
/// "killed by" part has its header at the strip's bottom edge too, but
/// above it only gameplay, whose edges do not span the panel (read over
/// the part left of the cards only, a wall edge passed on real gameplay).
const STRIP_EDGE_X: (i32, i32) = (1530, 1896);
/// Shortest and tallest strip (measured: 18 px).
const STRIP_H: (i32, i32) = (15, 21);
/// Where the strip's bottom edge may lie relative to the first card's
/// top: measured 40 to 45 px above it on real recaps, 48 px in the
/// synthetic ones, with a margin of three rows. The same window one card
/// pitch higher is searched next, for a first card that was missed.
const STRIP_BOTTOM: (i32, i32) = (-51, -37);
/// The strip's band edges must each score at least this.
const MIN_STRIP: f32 = 0.5;
/// The strip's top edge may instead be a straight line over the part
/// left of the cards only: over the card art its contrast can fade (a
/// real top edge measured 1.0 on the left part and 0.48 across the
/// panel without colour, 0.52 with it). The bottom edge must still span
/// the panel.
const MIN_STRIP_TOP_LEFT: f32 = 0.8;
/// Rows of the killer's name slot around the strip found (a margin of
/// three rows on each side).
const STRIP_MARGIN: i32 = 3;
/// Most cards read.
const MAX_CARDS: usize = 8;
/// Half the height of the rows the gate reads of the panel's edges around
/// the centre (every recap covers at least y 389..691).
const GATE_HALF: i32 = 90;
/// The rows in which the gate looks for the cards' outlines. Measured on
/// real recaps: the cards span y 340..630 in a five-card report shown
/// while alive and y 460..692 under the "killed by" part with four cards
/// (a fifth card would end near 715). The weapon list below (from y 730)
/// has vertical edges near the outlines' columns in every frame.
const GATE_CARDS: (i32, i32) = (320, 720);
/// Half the width of the strips sampled around each card outline.
const OUTLINE_HALF: i32 = 6;

/// The gate of the death recap: the panel's left and right edges around
/// the panel's vertical centre, which every recap covers whatever its
/// height, and the cards' outlines wherever a card lies (a combat report
/// with one card covers only 50 rows; overlays such as the spectator's
/// outline effect can hide some edges, and the translucent panel's edges
/// fade over a dark background).
pub const GATE: GateSpec = GateSpec {
    region: Region::DeathRecap,
    anchor: Anchor::Right,
    vertical: &[
        (PANEL_X0, PANEL_CY - GATE_HALF, PANEL_CY + GATE_HALF),
        (PANEL_X1, PANEL_CY - GATE_HALF, PANEL_CY + GATE_HALF),
    ],
    outlines: &[CARD_X.0, CARD_X.1],
    outline_span: GATE_CARDS,
    outline_rows: CARD_H.0,
    horizontal: &[],
    slide: 0,
    threshold: GATE_THRESHOLD,
};

/// The native zone the detector reads (reference pixels,
/// right-anchored).
pub const ZONE: RectI = RectI {
    x0: PANEL_X0 - OUTLINE_HALF - 2,
    y0: PANEL_Y.0,
    x1: PANEL_X1 + 4,
    y1: PANEL_Y.1,
};

/// Reads the name slots of the death recap.
#[derive(Debug, Clone, Default)]
pub struct RecapReader {
    panel: RefPlane,
    left: RefPlane,
    right: RefPlane,
    card: RefPlane,
    slot: RefPlane,
    hits: Vec<bool>,
    ink: InkScratch,
}

impl RecapReader {
    /// A reader.
    pub fn new() -> Self {
        Self::default()
    }

    /// The name slots in `view` (a native crop containing [`ZONE`]), or
    /// `None` if no combat-report card is found.
    pub fn read(&mut self, view: &FrameView<'_>, map: &ScreenMap) -> Option<Vec<Slot>> {
        let strip = |x: i32| RectI {
            x0: x - OUTLINE_HALF,
            y0: PANEL_Y.0,
            x1: x + OUTLINE_HALF,
            y1: PANEL_Y.1,
        };
        self.panel
            .sample(view, map, Anchor::Right, strip(PANEL_X0), 1);
        self.left
            .sample(view, map, Anchor::Right, strip(CARD_X.0), 1);
        self.right
            .sample(view, map, Anchor::Right, strip(CARD_X.1), 1);
        let mut cards = self.cards(view, map);
        let first = *cards.first()?;
        let strip = self.strip(view, map, first.0);
        let strip_bottom = strip.map_or(first.0 + STRIP_BOTTOM.0 - STRIP_H.1, |s| s.1);
        self.fill(&mut cards, strip_bottom);
        let mut slots = Vec::with_capacity(cards.len() + 1);
        if let Some((y0, y1)) = strip {
            let rect = RectI {
                x0: STRIP_X.0,
                y0,
                x1: STRIP_X.1,
                y1,
            };
            slots.push(Slot {
                kind: NameKind::RecapName,
                rect,
                ink: self.has_ink(view, map, rect),
            });
        }
        for &(top, bottom) in &cards {
            // The ink test reads the card's lower half only (the margin
            // holds the outline).
            let half = RectI {
                x0: CARD_X.0 + 1,
                y0: i32::midpoint(top, bottom),
                x1: CARD_X.1 - 1,
                y1: bottom,
            };
            slots.push(Slot {
                kind: NameKind::RecapName,
                rect: RectI {
                    y0: half.y0 - CARD_MARGIN.0,
                    y1: half.y1 + CARD_MARGIN.1,
                    ..half
                },
                ink: self.has_ink(view, map, half),
            });
        }
        Some(slots)
    }

    /// The rows of the killer's name slot: the strip found above the
    /// first card (top `first`) with a margin, or `None` (a combat report
    /// without the "killed by" part, or a strip too faint to find).
    fn strip(&mut self, view: &FrameView<'_>, map: &ScreenMap, first: i32) -> Option<(i32, i32)> {
        let area = RectI {
            x0: STRIP_EDGE_X.0 - 2,
            y0: first - CARD_PITCH + STRIP_BOTTOM.0 - STRIP_H.1 - 3,
            x1: STRIP_EDGE_X.1 + 2,
            y1: first + STRIP_BOTTOM.1 + 3,
        };
        self.card.sample(view, map, Anchor::Right, area, 1);
        let at = |y| edges::horizontal_at_y(&self.card, y, STRIP_EDGE_X.0, STRIP_EDGE_X.1);
        let top_at = |y| {
            let left = edges::horizontal_at_y(&self.card, y, STRIP_EDGE_X.0, CARD_X.0 - 2);
            if left >= MIN_STRIP_TOP_LEFT {
                at(y).max(MIN_STRIP)
            } else {
                at(y)
            }
        };
        // The best band whose bottom edge lies in the window `dy` above
        // the first card: `(score, top, bottom)`.
        let band = |dy: i32| {
            let mut best: Option<(f32, i32, i32)> = None;
            for bottom in first + dy + STRIP_BOTTOM.0..=first + dy + STRIP_BOTTOM.1 {
                let b = at(bottom);
                if b < MIN_STRIP {
                    continue;
                }
                for h in STRIP_H.0..=STRIP_H.1 {
                    let score = b.min(top_at(bottom - h));
                    if score >= MIN_STRIP && best.is_none_or(|(s, _, _)| score > s) {
                        best = Some((score, bottom - h, bottom));
                    }
                }
            }
            best
        };
        band(0)
            .or_else(|| band(-CARD_PITCH))
            .map(|(_, top, bottom)| (top - STRIP_MARGIN, bottom + STRIP_MARGIN))
    }

    /// Adds the cards on the row pitch that the strict test missed: between
    /// the cards found, and one pitch above and below them, as long as
    /// both outlines hold in most rows. `strip_bottom` limits the search
    /// upward.
    fn fill(&self, cards: &mut Vec<(i32, i32)>, strip_bottom: i32) {
        let (Some(&(first, _)), Some(&(last, _))) = (cards.first(), cards.last()) else {
            return;
        };
        let mut tops = Vec::new();
        // Between found cards.
        for pair in cards.windows(2) {
            let mut t = pair[0].0 + CARD_PITCH;
            while t + CARD_PITCH / 2 < pair[1].0 {
                tops.push(t);
                t += CARD_PITCH;
            }
        }
        tops.retain(|&t| self.outlines_hold(t));
        // Above the first and below the last one.
        let room = |tops: &Vec<i32>| cards.len() + tops.len() < MAX_CARDS;
        let mut t = first - CARD_PITCH;
        while t > strip_bottom && room(&tops) && self.outlines_hold(t) {
            tops.push(t);
            t -= CARD_PITCH;
        }
        let mut t = last + CARD_PITCH;
        while t + FILL_H <= PANEL_Y.1 && room(&tops) && self.outlines_hold(t) {
            tops.push(t);
            t += CARD_PITCH;
        }
        cards.extend(tops.into_iter().map(|t| (t, t + FILL_H)));
        cards.sort_unstable();
        cards.truncate(MAX_CARDS);
    }

    /// Whether the left and right card outlines are thin lines in at least
    /// [`MIN_FILL`] of the rows of a card at top `top`.
    fn outlines_hold(&self, top: i32) -> bool {
        let y0 = self.left.rect().y0;
        let (a, b) = (top - y0, top + FILL_H - y0);
        [&self.left, &self.right].into_iter().all(|plane| {
            let column = best_column(plane, a, b, line_at);
            let n = (a..b).filter(|&y| line_at(plane, column, y)).count();
            n as f32 >= MIN_FILL * (b - a) as f32
        })
    }

    /// The cards `(top, bottom)` in reference rows, top to bottom: runs of
    /// rows where the left outline is an edge, of a card's height, whose
    /// other outlines and the panel edge beside them hold too.
    fn cards(&mut self, view: &FrameView<'_>, map: &ScreenMap) -> Vec<(i32, i32)> {
        let rows = self.left.height() as i32;
        // Per row, the outline may sit in any column near its place (the
        // card art above the report can hold its own lines elsewhere).
        self.hits.clear();
        self.hits
            .extend((0..rows).map(|y| (-2..=2).any(|d| line_at(&self.left, OUTLINE_HALF + d, y))));
        let y0 = self.left.rect().y0;
        let mut cards = Vec::new();
        let mut start: Option<i32> = None;
        let mut last_hit = 0;
        for y in 0..=rows {
            let hit = self.hits.get(y as usize).copied().unwrap_or(false);
            if hit {
                start.get_or_insert(y);
                last_hit = y;
                continue;
            }
            let Some(s) = start else {
                continue;
            };
            if y - last_hit > CARD_GAP || y == rows {
                let end = last_hit + 1;
                let len = end - s;
                for (top, bottom) in split_run(y0 + s, len) {
                    if cards.len() < MAX_CARDS
                        && let Some(card) = self.card_at(view, map, top, bottom)
                    {
                        cards.push(card);
                    }
                }
                start = None;
            }
        }
        cards
    }

    /// The card in reference rows about `top..bottom`, with its rows
    /// refined to its top and bottom outlines, if they hold a card: its
    /// right, top and bottom outlines are thin lines and its inside is
    /// mostly smooth; and either the panel's left edge beside it is a
    /// straight edge, or all its outlines are strong (a background as dark
    /// as the panel hides the panel's edge).
    fn card_at(
        &mut self,
        view: &FrameView<'_>,
        map: &ScreenMap,
        top: i32,
        bottom: i32,
    ) -> Option<(i32, i32)> {
        let y0 = self.left.rect().y0;
        let share = |plane: &RefPlane, test: fn(&RefPlane, i32, i32) -> bool| {
            let (a, b) = (top - y0, bottom - y0);
            let column = best_column(plane, a, b, test);
            let n = (a..b).filter(|&y| test(plane, column, y)).count();
            n as f32 / (b - a).max(1) as f32
        };
        let right = share(&self.right, line_at);
        if right < MIN_OUTLINE {
            return None;
        }
        let panel = share(&self.panel, edge_at);
        let area = RectI {
            x0: CARD_X.0 - 4,
            y0: top - LINE_SEARCH - 3,
            x1: CARD_X.1 + 4,
            y1: bottom + LINE_SEARCH + LINE_W + 3,
        };
        self.card.sample(view, map, Anchor::Right, area, 1);
        let (x0, x1) = (CARD_X.0 + 4, CARD_X.1 - 4);
        // A thin line starting at row `y`: an edge into it and one out of
        // it.
        let thin = |y: i32| {
            let at = |y| edges::horizontal_at_y(&self.card, y, x0, x1);
            let out = (1..=LINE_W).map(|w| at(y + w)).fold(0.0, f32::max);
            at(y).min(out)
        };
        // The best line near row `y`: `(score, row)`.
        let best = |y: i32| {
            (-LINE_SEARCH..=LINE_SEARCH)
                .map(|d| (thin(y + d), y + d))
                .fold((0.0, y), |b, c| if c.0 > b.0 { c } else { b })
        };
        let (top_line, card_top) = best(top);
        let (bottom_line, bottom_row) = best(bottom - 1);
        let outlined = if panel >= MIN_OUTLINE {
            top_line >= MIN_LINE && bottom_line >= MIN_LINE
        } else {
            right.min(top_line).min(bottom_line) >= MIN_STRONG
        };
        let card = (card_top, bottom_row + 1);
        (outlined && card.1 > card.0 && self.smooth_inside(card.0, card.1)).then_some(card)
    }

    /// Whether the card's inside right of the portrait is mostly smooth:
    /// at most [`MAX_INSIDE_EDGES`] of its horizontally neighbouring cells
    /// differ by [`edges::MIN_STEP`] or more (a weapon icon and the name
    /// on a flat card, not the textured card art above the report).
    fn smooth_inside(&self, top: i32, bottom: i32) -> bool {
        let (cx0, cy0) = self.card.cell(INSIDE_X.0, top + 3);
        let (cx1, cy1) = self.card.cell(INSIDE_X.1, bottom - 3);
        let mut pairs = 0u32;
        let mut steps = 0u32;
        for cy in cy0..cy1 {
            for cx in cx0..cx1 - 1 {
                let (a, b) = (self.card.at(cx, cy), self.card.at(cx + 1, cy));
                let d = (0..3).map(|c| (a[c] - b[c]).abs()).max().unwrap_or(0);
                pairs += 1;
                steps += u32::from(d >= edges::MIN_STEP);
            }
        }
        pairs > 0 && (steps as f32) <= MAX_INSIDE_EDGES * pairs as f32
    }

    /// Whether the reference rectangle shows ink inside its outer two
    /// pixels (outlines and band edges are not ink).
    fn has_ink(&mut self, view: &FrameView<'_>, map: &ScreenMap, r: RectI) -> bool {
        self.slot.sample(view, map, Anchor::Right, r, 1);
        let inner = RectI {
            x0: r.x0 + 2,
            y0: r.y0 + 2,
            x1: r.x1 - 2,
            y1: r.y1 - 2,
        };
        has_ink(&self.slot, inner, &mut self.ink)
    }
}

/// The cards a run of outline rows starting at `top` with `len` rows can
/// hold: one card of a card's height, or several cards on the row pitch
/// whose outlines were joined by marks between them (the digits next to
/// the cards can bridge the gap). Anything else holds no card.
fn split_run(top: i32, len: i32) -> Vec<(i32, i32)> {
    if (CARD_H.0..=CARD_H.1).contains(&len) {
        return vec![(top, top + len)];
    }
    let gap = CARD_PITCH - FILL_H;
    let k = (len + gap + CARD_PITCH / 2) / CARD_PITCH;
    if k < 2 || (len - (k * CARD_PITCH - gap)).abs() > SPLIT_TOLERANCE {
        return Vec::new();
    }
    (0..k)
        .map(|i| {
            let t = top + i * CARD_PITCH;
            (t, t + FILL_H)
        })
        .collect()
}

/// The column within two cells of an outline strip's centre that passes
/// `test` in the most rows of `y0..y1`.
fn best_column(plane: &RefPlane, y0: i32, y1: i32, test: fn(&RefPlane, i32, i32) -> bool) -> i32 {
    (-2..=2)
        .map(|d| {
            let x = OUTLINE_HALF + d;
            ((y0..y1).filter(|&y| test(plane, x, y)).count(), x)
        })
        .max()
        .map_or(OUTLINE_HALF, |(_, x)| x)
}

#[cfg(test)]
mod tests {
    use blurrd_testkit::hud::killfeed::Team;
    use blurrd_testkit::hud::recap::{RecapSpec, SECTIONS};
    use blurrd_testkit::hud::screen::ScreenTiming;
    use blurrd_testkit::{Background, FontSet, Layout, Palette, Scene};

    use super::{CARD_PITCH, CARD_X, FILL_H, GATE, RecapReader, STRIP_X, split_run};
    use crate::image::RgbView;
    use crate::screens::gate::Gates;
    use crate::screens::sample::{Anchor, FrameView, ScreenMap};

    #[test]
    fn finds_the_strip_when_its_top_edge_fades_over_the_card_art() {
        let spec = RecapSpec {
            killer: "Killer Name".to_owned(),
            killer_team: Team::Enemy,
            agent: "Agent".to_owned(),
            report: vec!["Opponent".to_owned()],
            timeline: 1,
            timing: ScreenTiming::instant(0, 4),
        };
        let strip_top = spec.top() + SECTIONS[0] + SECTIONS[1];
        let scene = Scene::new(
            Layout::reference(),
            Background::noise(3),
            Palette::DEFAULT,
            2,
        )
        .with_recap(&FontSet::latin(), spec);
        let mut frame = scene.iter().next().unwrap().frame;
        // Over the card art (right of the strip's name part) the card art
        // takes the strip's colour: no edge there.
        for y in strip_top - SECTIONS[1]..strip_top {
            for x in CARD_X.0 - 1..1900 {
                let strip = frame.pixel(x, strip_top + 1).unwrap();
                frame.set(x, y, strip);
            }
        }
        let view = FrameView::native(
            RgbView::new(
                frame.data(),
                frame.width(),
                frame.height(),
                3 * frame.width() as usize,
            )
            .unwrap(),
        );
        let slots = RecapReader::new()
            .read(&view, &ScreenMap::REFERENCE)
            .unwrap();
        let killer = slots
            .iter()
            .find(|s| s.rect.x0 == STRIP_X.0)
            .expect("the killer's name slot");
        assert!(killer.ink);
        assert!(killer.rect.y0 <= strip_top && killer.rect.y1 >= strip_top + SECTIONS[2]);
    }

    #[test]
    fn the_gate_holds_a_one_card_report_over_dark_changing_scenes() {
        // A combat report with one card, without the "killed by" part (the
        // report shown while alive), over dark scenes that change between
        // frames, at full brightness and darkened like a gamma filter.
        let spec = RecapSpec {
            killer: "Killer Name".to_owned(),
            killer_team: Team::Enemy,
            agent: "Agent".to_owned(),
            report: vec!["Opponent".to_owned()],
            timeline: 4,
            timing: ScreenTiming::instant(0, 4),
        };
        let report_top = spec.top() + SECTIONS[0] + SECTIONS[1] + SECTIONS[2];
        let mut gates = Gates::new();
        let mut reader = RecapReader::new();
        for seed in 0..6 {
            let background = Background::Noise {
                seed,
                base: [44, 46, 50],
                amp: 30.0,
                cell: 16,
            };
            let scene = |recap: bool| {
                let s = Scene::new(Layout::reference(), background, Palette::DEFAULT, 2);
                let s = if recap {
                    s.with_recap(&FontSet::latin(), spec.clone())
                } else {
                    s
                };
                s.render(1).frame
            };
            let plain = scene(false);
            for dark in [1.0, 0.5] {
                let mut frame = scene(true);
                for y in 0..report_top {
                    for x in 1500..1920 {
                        frame.set(x, y, plain.pixel(x, y).unwrap());
                    }
                }
                frame.multiply_rect(frame.bounds(), dark);
                let view = FrameView::native(
                    RgbView::new(
                        frame.data(),
                        frame.width(),
                        frame.height(),
                        3 * frame.width() as usize,
                    )
                    .unwrap(),
                );
                // At the gate's step the native view samples the same cell
                // means as the quarter-scale frame.
                let signal = gates.evaluate(&GATE, &view, &ScreenMap::REFERENCE);
                assert!(signal.candidate, "seed {seed} x{dark}: {}", signal.score);
                let slots = reader.read(&view, &ScreenMap::REFERENCE).unwrap();
                assert!(
                    slots.iter().any(|s| s.rect.x0 > CARD_X.0 && s.ink),
                    "seed {seed} x{dark}: {slots:?}"
                );
            }
        }
    }

    #[test]
    fn the_gate_finds_outlines_spread_over_two_view_pixels() {
        // The quarter-scale frame of an open combat report over a screen
        // of the panel's tone (the panel's edges vanish): four cards whose
        // one-pixel outlines a downscaled recording spreads over two view
        // pixels at half contrast.
        for (w, h) in [(1280, 720), (1600, 900), (1920, 1080), (2560, 1440)] {
            let map = ScreenMap::wide(w, h);
            let (qw, qh) = (w / 4, h / 4);
            let mut data = vec![26u8; (3 * qw * qh) as usize];
            let column = |x: i32| (map.x(x as f32, Anchor::Right) * 0.25) as usize;
            let row = |y: i32| (map.y(y as f32) * 0.25) as usize;
            for k in 0..4 {
                let top = 470 + CARD_PITCH * k;
                for y in row(top)..row(top + FILL_H) {
                    for x in [column(CARD_X.0), column(CARD_X.1)] {
                        let i = 3 * (y * qw as usize + x);
                        data[i..i + 6].fill(88);
                    }
                }
            }
            let view =
                FrameView::scaled(RgbView::new(&data, qw, qh, 3 * qw as usize).unwrap(), 0.25);
            let signal = Gates::new().evaluate(&GATE, &view, &map);
            assert!(signal.candidate, "{w}x{h}: {}", signal.score);
        }
    }

    #[test]
    fn splits_joined_runs_with_marks_at_their_ends() {
        let tops = |cards: Vec<(i32, i32)>| cards.into_iter().map(|c| c.0).collect::<Vec<_>>();
        // Four cards (224 rows) joined with seven rows of marks below.
        let cards = split_run(457, 4 * CARD_PITCH - (CARD_PITCH - FILL_H) + 7);
        assert_eq!(tops(cards), [457, 515, 573, 631]);
        // A run between one and two cards holds none.
        assert_eq!(split_run(457, 85), vec![]);
    }
}
