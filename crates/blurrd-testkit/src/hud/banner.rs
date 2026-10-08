//! ACE / clutch banners in the top centre.
//!
//! Geometry in reference pixels from the port spec (section 4): a framed
//! box with a large title, bright corner ticks at rows 280..288 (x 698..716
//! and 1204..1222), and under it a grey plate at y 299..328 whose player
//! name (12 px capitals, baseline 319) slides in from the right, slowing
//! from about 50 px per frame, and settles centred on x 960.

use blurrd_core::{NameKind, RectI, Region};

use crate::font::FontSet;
use crate::layout::{Anchor, Layout};
use crate::paint::{Label, NameDraw, Painter, rect_f};
use crate::palette::TEXT_WHITE;
use crate::truth::{ContentKind, ItemId};

/// Capital height of the banner name.
pub const NAME_CAP: f32 = 12.0;
/// Baseline of the banner name.
pub const NAME_BASELINE: f32 = 319.0;
/// The plate under the banner box.
pub const PLATE: RectI = RectI {
    x0: 725,
    y0: 299,
    x1: 1195,
    y1: 328,
};
/// Frames the box takes to appear before the name starts sliding.
const INTRO: u32 = 4;
/// Item id of the banner name.
const ID: u32 = 2000;

/// A banner episode.
#[derive(Debug, Clone, PartialEq)]
pub struct BannerSpec {
    /// Title in the box, for example "ACE" or "CLUTCH".
    pub title: String,
    /// The player name on the plate.
    pub name: String,
    /// Frame in which the box appears.
    pub start: u32,
    /// Frames the name takes to slide in.
    pub slide_frames: u32,
    /// Distance the name slides, in reference pixels.
    pub slide_px: f32,
    /// Frames the settled banner stays.
    pub hold: u32,
    /// Frames everything takes to fade out.
    pub fade: u32,
}

impl BannerSpec {
    /// A banner with the measured animation: a 400 px slide over 16 frames
    /// (50 px in the first frame), held 2.5 s, fading over 10 frames.
    pub fn new(title: &str, name: &str, start: u32) -> Self {
        Self {
            title: title.to_owned(),
            name: name.to_owned(),
            start,
            slide_frames: 16,
            slide_px: 400.0,
            hold: 150,
            fade: 10,
        }
    }

    /// The first frame after the slide (the name is centred from here on).
    pub fn settle(&self) -> u32 {
        self.start + INTRO + self.slide_frames
    }

    /// The last frame in which anything is visible.
    pub fn end(&self) -> u32 {
        self.settle() + self.hold + self.fade
    }
}

/// A banner rendered for one layout.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Banner {
    spec: BannerSpec,
    title: Label,
    name: Label,
    /// Half width of the slot: sixteen "W" plus a margin.
    slot_half: f32,
}

impl Banner {
    pub(crate) fn new(fonts: &FontSet, layout: &Layout, spec: BannerSpec) -> Self {
        let title = Label::new(fonts, layout, &spec.title, 60.0);
        let name = Label::new(fonts, layout, &spec.name, NAME_CAP);
        let widest = Label::new(fonts, layout, &"W".repeat(16), NAME_CAP);
        Self {
            spec,
            title,
            name,
            slot_half: widest.width * 0.5 + 6.0,
        }
    }

    pub(crate) fn paint(&self, p: &mut Painter<'_>, frame: u32) {
        let s = &self.spec;
        if frame < s.start || frame > s.end() {
            return;
        }
        let t = frame - s.start;
        let fade_from = s.settle() + s.hold;
        let alpha = if t < INTRO {
            (t + 1) as f32 / INTRO as f32
        } else if frame >= fade_from {
            1.0 - (frame - fade_from + 1) as f32 / (s.fade + 1) as f32
        } else {
            1.0
        };
        let a = Anchor::Center;
        // Box with a thin frame and the bright corner ticks.
        let frame_rect = RectI {
            x0: 705,
            y0: 135,
            x1: 1215,
            y1: 283,
        };
        p.fill(frame_rect, a, [20, 24, 30], 0.25 * alpha);
        for edge in [
            RectI {
                y1: frame_rect.y0 + 2,
                ..frame_rect
            },
            RectI {
                y0: frame_rect.y1 - 2,
                ..frame_rect
            },
            RectI {
                x1: frame_rect.x0 + 2,
                ..frame_rect
            },
            RectI {
                x0: frame_rect.x1 - 2,
                ..frame_rect
            },
        ] {
            p.fill(edge, a, [230, 230, 230], 0.7 * alpha);
        }
        for x0 in [698, 1204] {
            p.fill(
                rect_f(x0 as f32, 280.0, (x0 + 18) as f32, 283.0),
                a,
                [252; 3],
                alpha,
            );
            let vx = if x0 < 960 { x0 } else { x0 + 15 };
            p.fill(
                rect_f(vx as f32, 280.0, (vx + 3) as f32, 288.0),
                a,
                [252; 3],
                alpha,
            );
        }
        let title_pen = (960.0 - self.title.width * 0.5, 240.0);
        p.label(
            ContentKind::BannerTitle,
            &self.title,
            title_pen,
            a,
            TEXT_WHITE,
            alpha,
            None,
        );
        p.fill(PLATE, a, [92, 92, 98], 0.55 * alpha);
        if t < INTRO {
            return;
        }
        let slide_t = (t - INTRO).min(s.slide_frames) as f32 / s.slide_frames.max(1) as f32;
        let offset = s.slide_px * (1.0 - slide_t) * (1.0 - slide_t);
        let pen = (960.0 - self.name.width * 0.5 + offset, NAME_BASELINE);
        let settled = rect_f(960.0 - self.slot_half, 300.0, 960.0 + self.slot_half, 327.0);
        // While sliding, the slot reaches from the settled slot to the
        // name's right end.
        let slot = if offset > 0.0 {
            rect_f(
                settled.x0 as f32,
                300.0,
                (pen.0 + self.name.width + 12.0).max(settled.x1 as f32),
                327.0,
            )
        } else {
            settled
        };
        p.name(
            NameDraw {
                id: ItemId(ID),
                region: Region::Banner,
                kind: NameKind::Banner,
                slot,
            },
            &self.name,
            pen,
            a,
            TEXT_WHITE,
            alpha,
            None,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::BannerSpec;

    #[test]
    fn timeline_is_consistent() {
        let s = BannerSpec::new("ACE", "Name", 100);
        assert_eq!(s.settle(), 120);
        assert_eq!(s.end(), 280);
        // First slide step is 50 px: 400 × (1 − (1 − 1/16)²).
        let first = 400.0 - 400.0 * (15.0f32 / 16.0).powi(2);
        assert!((first - 48.4).abs() < 0.1);
    }
}
