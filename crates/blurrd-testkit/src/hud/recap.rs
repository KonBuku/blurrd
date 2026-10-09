//! The death recap: the "killed by" panel and the combat report.
//!
//! Geometry in reference pixels, measured on real recordings
//! (right-anchored): a panel from x 1525 to 1900, centred vertically on
//! y 540, whose height follows its content. From the top: the "KILLED
//! BY" header (32 px), the killer's card art (82 px, the killer's agent
//! portrait on the left), a strip in the killer's team colour (18 px)
//! with the killer's name on the left (pen x 1529, 8 px capitals) and
//! the agent name on the right, the "COMBAT REPORT" header (44 px), one
//! 58 px row per opponent, one 48 px timeline row per event and a 20 px
//! footer. Each report row has an outlined card at x 1638..1788, 4 px
//! inside the row's top and bottom, with the opponent's portrait and the
//! name in its lower half (centred on x 1737, baseline 44 px below the
//! card top), damage numbers left and right of the card. One report row and one timeline row give the
//! measured panel at y 389..691.

use blurrd_core::{NameKind, RectI, Region};

use super::killfeed::Team;
use super::screen::{ScreenTiming, outline};
use crate::font::FontSet;
use crate::layout::{Anchor, Layout};
use crate::paint::{Label, NameDraw, Painter};
use crate::palette::{Palette, TEXT_GREY, TEXT_WHITE};
use crate::truth::{ContentKind, ItemId};

/// Left edge of the panel.
pub const PANEL_X0: i32 = 1525;
/// Right edge of the panel.
pub const PANEL_X1: i32 = 1900;
/// Vertical centre of the panel.
pub const PANEL_CY: i32 = 540;
/// Heights of the sections above the report rows: header, card art,
/// strip, report header.
pub const SECTIONS: [i32; 4] = [32, 82, 18, 44];
/// Height (and pitch) of a combat-report row.
pub const REPORT_H: i32 = 58;
/// Height of a timeline row.
pub const TIMELINE_H: i32 = 48;
/// Height of the footer.
pub const FOOTER_H: i32 = 20;
/// The killer name's slot in the strip (x range).
pub const STRIP_NAME_X: (i32, i32) = (1527, 1712);
/// The opponent card of a report row (x range, outline included).
pub const CARD_X: (i32, i32) = (1638, 1788);
/// Rows between a report row's edges and its card's outline (measured:
/// row 455..513, card 459..509).
pub const CARD_INSET: i32 = 4;
/// Capital height of the recap names.
pub const NAME_CAP: f32 = 8.0;
/// Most report rows drawn.
pub const MAX_REPORT_ROWS: usize = 5;

/// Item id of the killer name; report names follow.
const ID_BASE: u32 = 6000;

/// A death recap shown over some frames.
#[derive(Debug, Clone, PartialEq)]
pub struct RecapSpec {
    /// The killer's name.
    pub killer: String,
    /// The killer's team (strip colour).
    pub killer_team: Team,
    /// The killer's agent, shown at the right of the strip (not a player
    /// name).
    pub agent: String,
    /// Opponents listed in the combat report (1 to
    /// [`MAX_REPORT_ROWS`]).
    pub report: Vec<String>,
    /// Timeline rows under the report.
    pub timeline: u8,
    /// When the recap is visible.
    pub timing: ScreenTiming,
}

impl RecapSpec {
    /// The panel's top edge (reference pixels) for its content.
    pub fn top(&self) -> i32 {
        let rows = self.report.len().clamp(1, MAX_REPORT_ROWS) as i32;
        let height = SECTIONS.iter().sum::<i32>()
            + REPORT_H * rows
            + TIMELINE_H * i32::from(self.timeline)
            + FOOTER_H;
        PANEL_CY - height / 2
    }
}

/// A death recap rendered for one layout.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Recap {
    spec: RecapSpec,
    header: Label,
    killer: Label,
    agent: Label,
    title: Label,
    report: Vec<Label>,
    numbers: Label,
}

impl Recap {
    pub(crate) fn new(fonts: &FontSet, layout: &Layout, spec: RecapSpec) -> Self {
        Self {
            header: Label::new(fonts, layout, "KILLED BY", 9.0),
            killer: Label::new(fonts, layout, &spec.killer, NAME_CAP),
            agent: Label::new(fonts, layout, &spec.agent, NAME_CAP),
            title: Label::new(fonts, layout, "COMBAT REPORT", 13.0),
            report: spec
                .report
                .iter()
                .take(MAX_REPORT_ROWS)
                .map(|name| Label::new(fonts, layout, name, NAME_CAP))
                .collect(),
            numbers: Label::new(fonts, layout, "159", 18.0),
            spec,
        }
    }

    pub(crate) fn paint(&self, p: &mut Painter<'_>, palette: &Palette, frame: u32) {
        let Some((alpha, offset)) = self.spec.timing.phase(frame) else {
            return;
        };
        let a = Anchor::Right;
        let top = self.spec.top() + offset.round() as i32;
        let rect = |x0: i32, y0: i32, x1: i32, y1: i32| RectI { x0, y0, x1, y1 };
        let [header_h, card_h, strip_h, title_h] = SECTIONS;
        let card_y = top + header_h;
        let strip_y = card_y + card_h;
        let title_y = strip_y + strip_h;
        let rows_y = title_y + title_h;
        let rows = self.report.len() as i32;
        let bottom =
            rows_y + REPORT_H * rows.max(1) + TIMELINE_H * i32::from(self.spec.timeline) + FOOTER_H;
        p.fill(
            rect(PANEL_X0, top, PANEL_X1, bottom),
            a,
            [24, 26, 30],
            0.72 * alpha,
        );
        let centre = f32::midpoint(PANEL_X0 as f32, PANEL_X1 as f32);
        p.label(
            ContentKind::ScreenText,
            &self.header,
            (centre - self.header.width * 0.5, (top + 21) as f32),
            a,
            TEXT_GREY,
            alpha,
            None,
        );
        p.portrait(rect(1526, card_y, 1607, strip_y), a, 61, alpha);
        p.portrait(rect(1607, card_y, PANEL_X1, strip_y), a, 67, 0.8 * alpha);
        p.fill(
            rect(PANEL_X0, strip_y, PANEL_X1, title_y),
            a,
            self.spec.killer_team.colour(palette),
            0.45 * alpha,
        );
        let strip_slot = rect(STRIP_NAME_X.0, strip_y + 1, STRIP_NAME_X.1, title_y);
        p.name(
            NameDraw {
                id: ItemId(ID_BASE),
                region: Region::DeathRecap,
                kind: NameKind::RecapName,
                slot: strip_slot,
            },
            &self.killer,
            (1529.0, (strip_y + 14) as f32),
            a,
            TEXT_WHITE,
            alpha,
            Some(strip_slot),
        );
        p.label(
            ContentKind::ScreenText,
            &self.agent,
            (1896.0 - self.agent.width, (strip_y + 14) as f32),
            a,
            TEXT_GREY,
            alpha,
            None,
        );
        p.label(
            ContentKind::ScreenText,
            &self.title,
            (1712.0 - self.title.width * 0.5, (title_y + 28) as f32),
            a,
            TEXT_WHITE,
            alpha,
            None,
        );
        for (k, label) in (0i32..).zip(&self.report) {
            self.paint_report_row(p, k, label, rows_y + REPORT_H * k, alpha);
        }
    }

    /// Report row `k` with its top at `row_top`: the outlined card with the
    /// opponent's portrait and name, damage numbers on both sides.
    fn paint_report_row(
        &self,
        p: &mut Painter<'_>,
        k: i32,
        label: &Label,
        row_top: i32,
        alpha: f32,
    ) {
        let a = Anchor::Right;
        let card_top = row_top + CARD_INSET;
        let card_bottom = row_top + REPORT_H - CARD_INSET;
        let card = RectI {
            x0: CARD_X.0,
            y0: card_top,
            x1: CARD_X.1,
            y1: card_bottom,
        };
        outline(p, card, a, [205, 205, 205], 0.85 * alpha);
        p.portrait(
            RectI {
                x0: CARD_X.0 + 3,
                y0: card_top + 3,
                x1: 1700,
                y1: card_bottom - 3,
            },
            a,
            71 + k as u64,
            alpha,
        );
        // Centred on the measured x, kept inside the card.
        let x = (1737.0 - label.width * 0.5)
            .min(CARD_X.1 as f32 - 3.0 - label.width)
            .max(CARD_X.0 as f32 + 3.0);
        let slot = RectI {
            x0: CARD_X.0 + 1,
            y0: i32::midpoint(card_top, card_bottom),
            x1: CARD_X.1 - 1,
            y1: card_bottom - 1,
        };
        p.name(
            NameDraw {
                id: ItemId(ID_BASE + 1 + k as u32),
                region: Region::DeathRecap,
                kind: NameKind::RecapName,
                slot,
            },
            label,
            (x, (card_top + 44) as f32),
            a,
            TEXT_WHITE,
            alpha,
            Some(slot),
        );
        for x in [1560.0, 1860.0] {
            let pen = (x - self.numbers.width * 0.5, (row_top + 38) as f32);
            p.label(
                ContentKind::Stat,
                &self.numbers,
                pen,
                a,
                TEXT_WHITE,
                alpha,
                None,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RecapSpec;
    use crate::hud::killfeed::Team;
    use crate::hud::screen::ScreenTiming;

    #[test]
    fn panel_matches_the_measured_positions() {
        let mut spec = RecapSpec {
            killer: "a".to_owned(),
            killer_team: Team::Enemy,
            agent: "b".to_owned(),
            report: vec!["c".to_owned()],
            timeline: 1,
            timing: ScreenTiming::instant(0, 1),
        };
        // One report row and one timeline row: y 389..691.
        assert_eq!(spec.top(), 389);
        spec.report = vec!["c".to_owned(); 4];
        spec.timeline = 2;
        // Four report rows and two timeline rows: measured top 284.
        assert!((spec.top() - 284).abs() <= 6, "{}", spec.top());
    }
}
