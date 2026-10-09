//! The spectated player's name: the "switch player" widget.
//!
//! Geometry in reference pixels, measured on real recordings
//! (left-anchored): while the recording player is dead and spectates a
//! teammate, the lower left shows a thin light vertical bar at x 30 over
//! rows 795..875, the teammate's agent portrait right of it (x 31..111),
//! the teammate's name (16 px capitals, rows 828..843, pen x 121) and
//! under it a mouse icon between "<" and ">" with the label "SWITCH
//! PLAYER" (x 122..271, rows 852..873). The widget sits on a dark
//! backdrop (measured luminance 9 to 21 around it). The name slot spans
//! x 116..420 (the longest names), rows 821..851.

use blurrd_core::{NameKind, RectI, Region};

use super::screen::ScreenTiming;
use crate::font::FontSet;
use crate::layout::{Anchor, Layout};
use crate::paint::{Label, NameDraw, Painter};
use crate::palette::{TEXT_GREY, TEXT_WHITE};
use crate::truth::{ContentKind, ItemId};

/// The vertical bar left of the portrait.
pub const BAR: RectI = RectI {
    x0: 30,
    y0: 795,
    x1: 31,
    y1: 875,
};
/// The agent portrait.
pub const PORTRAIT: RectI = RectI {
    x0: 31,
    y0: 795,
    x1: 111,
    y1: 875,
};
/// The name slot.
pub const NAME_SLOT: RectI = RectI {
    x0: 116,
    y0: 821,
    x1: 420,
    y1: 851,
};
/// Capital height of the name.
pub const NAME_CAP: f32 = 16.0;
/// Left edge of the name's pen.
pub const NAME_X: f32 = 121.0;
/// Baseline of the name.
pub const NAME_BASELINE: f32 = 844.0;

/// The dark backdrop behind the widget.
const BACKDROP: RectI = RectI {
    x0: 0,
    y0: 770,
    x1: 440,
    y1: 900,
};
/// The mouse icon between "<" and ">".
const MOUSE: RectI = RectI {
    x0: 135,
    y0: 853,
    x1: 145,
    y1: 873,
};
/// Item id of the spectated name.
const ID: u32 = 7000;

/// A "switch player" widget over some frames.
#[derive(Debug, Clone, PartialEq)]
pub struct SpectatingSpec {
    /// The spectated player's name.
    pub name: String,
    /// When the widget is visible.
    pub timing: ScreenTiming,
}

/// A "switch player" widget rendered for one layout.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Spectating {
    spec: SpectatingSpec,
    arrows: [Label; 2],
    label: Label,
    name: Label,
}

impl Spectating {
    pub(crate) fn new(fonts: &FontSet, layout: &Layout, spec: SpectatingSpec) -> Self {
        Self {
            arrows: ["<", ">"].map(|a| Label::new(fonts, layout, a, 8.0)),
            label: Label::new(fonts, layout, "SWITCH PLAYER", 8.0),
            name: Label::new(fonts, layout, &spec.name, NAME_CAP),
            spec,
        }
    }

    pub(crate) fn paint(&self, p: &mut Painter<'_>, frame: u32) {
        let Some((alpha, offset)) = self.spec.timing.phase(frame) else {
            return;
        };
        let a = Anchor::Left;
        let dy = offset.round() as i32;
        let shift = |r: RectI| RectI {
            y0: r.y0 + dy,
            y1: r.y1 + dy,
            ..r
        };
        p.fill(BACKDROP, a, [14, 14, 16], 0.85 * alpha);
        p.fill(shift(BAR), a, [235, 235, 235], alpha);
        p.portrait(shift(PORTRAIT), a, 83, alpha);
        let slot = shift(NAME_SLOT);
        p.name(
            NameDraw {
                id: ItemId(ID),
                region: Region::Spectating,
                kind: NameKind::Spectated,
                slot,
            },
            &self.name,
            (NAME_X, NAME_BASELINE + dy as f32),
            a,
            TEXT_WHITE,
            alpha,
            Some(slot),
        );
        let baseline = (867 + dy) as f32;
        for (label, x) in self.arrows.iter().zip([123.0, 150.0]) {
            p.label(
                ContentKind::ScreenText,
                label,
                (x, baseline),
                a,
                TEXT_GREY,
                alpha,
                None,
            );
        }
        p.fill(shift(MOUSE), a, [200, 220, 220], alpha);
        p.label(
            ContentKind::ScreenText,
            &self.label,
            (166.0, baseline),
            a,
            TEXT_WHITE,
            alpha,
            None,
        );
    }
}
