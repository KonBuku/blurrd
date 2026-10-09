//! Match-end, MVP and round-end screens.
//!
//! The local test media contain no match-end or MVP screen (decision
//! D5), so [`MatchEndKind::Victory`] and [`MatchEndKind::Defeat`] are
//! stand-ins in the style of the measured surfaces (centre-anchored): the
//! game dimmed behind a large title and one outlined card per player of
//! the team, 220 px
//! wide on a 236 px pitch from x 372, at y 400..760, with the agent art
//! above a dark name strip at y 712..746 (11 px capitals, centred on the
//! card, baseline 734). The MVP card carries a "MATCH MVP" label above
//! it. The name slot of a card is its strip.
//!
//! [`MatchEndKind::RoundEnd`] is the round-end banner as measured on real
//! recordings: a framed box at x 705..1215, y 135..283 with a large
//! result word and a reason line ("ENEMY TEAM ELIMINATED"), sliding in
//! from the right. It shows no player name: the screen detectors must
//! not box it.

use blurrd_core::{NameKind, RectI, Region};

use super::screen::{ScreenTiming, outline};
use crate::font::FontSet;
use crate::layout::{Anchor, Layout};
use crate::paint::{Label, NameDraw, Painter, rect_f};
use crate::palette::{Palette, TEXT_GREY, TEXT_WHITE};
use crate::truth::{ContentKind, ItemId};

/// Left edge of the first card.
pub const CARD_X0: i32 = 372;
/// Width of a card.
pub const CARD_W: i32 = 220;
/// Distance between card left edges.
pub const CARD_PITCH: i32 = 236;
/// Top and bottom of the cards.
pub const CARD_Y: (i32, i32) = (400, 760);
/// The name strip of a card (y range).
pub const STRIP_Y: (i32, i32) = (712, 746);
/// Capital height of the names.
pub const NAME_CAP: f32 = 11.0;
/// Most cards (a team of five).
pub const MAX_CARDS: usize = 5;
/// The round-end banner box.
pub const ROUND_BOX: RectI = RectI {
    x0: 705,
    y0: 135,
    x1: 1215,
    y1: 283,
};

/// Item ids of match-end names start here.
const ID_BASE: u32 = 8000;

/// Which screen is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MatchEndKind {
    /// The match was won: "VICTORY" and the team's cards.
    Victory,
    /// The match was lost: "DEFEAT" and the team's cards.
    Defeat,
    /// The round-end banner (no names; a decoy).
    RoundEnd,
}

/// A match-end, MVP or round-end screen over some frames.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchEndSpec {
    /// Which screen.
    pub kind: MatchEndKind,
    /// The players' names on the cards (ignored for
    /// [`MatchEndKind::RoundEnd`]).
    pub names: Vec<String>,
    /// The card with the MVP label.
    pub mvp: usize,
    /// When the screen is visible.
    pub timing: ScreenTiming,
}

/// A match-end screen rendered for one layout.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MatchEnd {
    spec: MatchEndSpec,
    title: Label,
    subtitle: Label,
    mvp: Label,
    names: Vec<Label>,
}

impl MatchEnd {
    pub(crate) fn new(fonts: &FontSet, layout: &Layout, spec: MatchEndSpec) -> Self {
        let (title, subtitle) = match spec.kind {
            MatchEndKind::Victory => ("VICTORY", "MATCH SUMMARY"),
            MatchEndKind::Defeat => ("DEFEAT", "MATCH SUMMARY"),
            MatchEndKind::RoundEnd => ("WON", "ENEMY TEAM ELIMINATED"),
        };
        Self {
            title: Label::new(fonts, layout, title, 60.0),
            subtitle: Label::new(fonts, layout, subtitle, 7.0),
            mvp: Label::new(fonts, layout, "MATCH MVP", 8.0),
            names: spec
                .names
                .iter()
                .take(MAX_CARDS)
                .map(|n| Label::new(fonts, layout, n, NAME_CAP))
                .collect(),
            spec,
        }
    }

    pub(crate) fn paint(&self, p: &mut Painter<'_>, palette: &Palette, frame: u32) {
        let Some((alpha, offset)) = self.spec.timing.phase(frame) else {
            return;
        };
        let a = Anchor::Center;
        if self.spec.kind == MatchEndKind::RoundEnd {
            self.paint_round_end(p, alpha, offset);
            return;
        }
        let dy = offset.round() as i32;
        // The screen dims the game behind it.
        p.fill(
            RectI {
                x0: 0,
                y0: 0,
                x1: 1920,
                y1: 1080,
            },
            a,
            [10, 12, 16],
            0.7 * alpha,
        );
        p.label(
            ContentKind::ScreenText,
            &self.title,
            (960.0 - self.title.width * 0.5, (300 + dy) as f32),
            a,
            TEXT_WHITE,
            alpha,
            None,
        );
        p.label(
            ContentKind::ScreenText,
            &self.subtitle,
            (960.0 - self.subtitle.width * 0.5, (340 + dy) as f32),
            a,
            TEXT_GREY,
            alpha,
            None,
        );
        for (k, label) in (0i32..).zip(&self.names) {
            self.paint_card(p, palette, k, label, dy, alpha);
        }
    }

    /// Card `k` with the player's agent art and name, shifted down by
    /// `dy`.
    fn paint_card(
        &self,
        p: &mut Painter<'_>,
        palette: &Palette,
        k: i32,
        label: &Label,
        dy: i32,
        alpha: f32,
    ) {
        let a = Anchor::Center;
        let x0 = CARD_X0 + CARD_PITCH * k;
        let card = RectI {
            x0,
            y0: CARD_Y.0 + dy,
            x1: x0 + CARD_W,
            y1: CARD_Y.1 + dy,
        };
        p.fill(card, a, palette.ally, 0.35 * alpha);
        outline(p, card, a, [210, 210, 210], 0.8 * alpha);
        p.portrait(
            RectI {
                x0: x0 + 6,
                y0: card.y0 + 6,
                x1: x0 + CARD_W - 6,
                y1: STRIP_Y.0 + dy,
            },
            a,
            91 + k as u64,
            alpha,
        );
        let strip = RectI {
            x0,
            y0: STRIP_Y.0 + dy,
            x1: x0 + CARD_W,
            y1: STRIP_Y.1 + dy,
        };
        p.fill(strip, a, [16, 18, 22], 0.8 * alpha);
        let centre = (x0 + CARD_W / 2) as f32;
        if k as usize == self.spec.mvp {
            let pen = (centre - self.mvp.width * 0.5, (CARD_Y.0 - 8 + dy) as f32);
            p.label(
                ContentKind::ScreenText,
                &self.mvp,
                pen,
                a,
                TEXT_WHITE,
                alpha,
                None,
            );
        }
        let slot = RectI {
            x0: x0 + 2,
            x1: x0 + CARD_W - 2,
            ..strip
        };
        p.name(
            NameDraw {
                id: ItemId(ID_BASE + k as u32),
                region: Region::MatchEnd,
                kind: NameKind::MatchEndName,
                slot,
            },
            label,
            (centre - label.width * 0.5, (734 + dy) as f32),
            a,
            TEXT_WHITE,
            alpha,
            Some(slot),
        );
    }

    /// The measured round-end banner: a framed box sliding in from the
    /// right with the result word and the reason line.
    fn paint_round_end(&self, p: &mut Painter<'_>, alpha: f32, offset: f32) {
        let a = Anchor::Center;
        let dx = (offset * 4.0).round() as i32;
        let b = RectI {
            x0: ROUND_BOX.x0 + dx,
            x1: ROUND_BOX.x1 + dx,
            ..ROUND_BOX
        };
        p.fill(b, a, [120, 190, 170], 0.35 * alpha);
        for x0 in [b.x0 - 7, b.x1 - 11] {
            p.fill(
                rect_f(x0 as f32, 280.0, (x0 + 18) as f32, 283.0),
                a,
                [252; 3],
                alpha,
            );
        }
        let cx = f32::midpoint(b.x0 as f32, b.x1 as f32);
        p.label(
            ContentKind::BannerTitle,
            &self.title,
            (cx - self.title.width * 0.5, 230.0),
            a,
            TEXT_WHITE,
            alpha,
            None,
        );
        p.label(
            ContentKind::ScreenText,
            &self.subtitle,
            (cx - self.subtitle.width * 0.5, 270.0),
            a,
            TEXT_WHITE,
            alpha,
            None,
        );
    }
}
