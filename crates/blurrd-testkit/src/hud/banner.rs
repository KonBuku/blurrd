//! ACE / clutch banners in the top centre.
//!
//! Geometry in reference pixels, measured on 25 recorded banners (port
//! spec section 4): a framed box with a large title, bright corner ticks at
//! rows 280..285 (x 704..717 and 1204..1217), and under it a light grey
//! plate at y 298..328, about 390 px wide and centred on the player name
//! (11 px capitals, baseline 318). Plate and name appear about 23 frames
//! after the box, slide in together from the right (first seen about
//! 220 px right of their place, slowing from about 45 px per frame) and
//! settle centred on x 960 ten frames later. At the end the name fades
//! out first, box and plate stay a little longer (measured: 12 to 26
//! frames), or everything is cut at once by the round end.
//!
//! [`UltReadySpec`] adds the "X ULT READY" notification, which shows in
//! the same place without box or plate and carries no player name (the
//! letter is the ability key).

use blurrd_core::{NameKind, RectI, Region};

use crate::font::FontSet;
use crate::layout::{Anchor, Layout};
use crate::paint::{Label, NameDraw, Painter, rect_f};
use crate::palette::TEXT_WHITE;
use crate::truth::{ContentKind, ItemId};

/// Capital height of the banner name.
pub const NAME_CAP: f32 = 11.0;
/// Baseline of the banner name.
pub const NAME_BASELINE: f32 = 318.0;
/// The settled plate under the banner box.
pub const PLATE: RectI = RectI {
    x0: 765,
    y0: 298,
    x1: 1155,
    y1: 328,
};
/// Baseline of the "ULT READY" notification (its text is centred on
/// y 296 to 298, measured on 21 reads in 3 recordings).
pub const ULT_READY_BASELINE: f32 = 302.0;
/// Frames the box takes to appear before the name starts sliding.
const INTRO: u32 = 23;
/// Frames box and plate stay after the name has faded out.
const OUTRO: u32 = 12;
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
    /// Frames the name takes to fade out.
    pub fade: u32,
}

impl BannerSpec {
    /// A banner with the measured animation: a 270 px slide over 10
    /// frames (eased, 51 px in the first frame), held 2.5 s, the name
    /// fading over 10 frames and box and plate gone 12 frames later.
    pub fn new(title: &str, name: &str, start: u32) -> Self {
        Self {
            title: title.to_owned(),
            name: name.to_owned(),
            start,
            slide_frames: 10,
            slide_px: 270.0,
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
        self.settle() + self.hold + self.fade + OUTRO
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

    /// Paints the box: frame, corner ticks and title.
    fn paint_box(&self, p: &mut Painter<'_>, alpha: f32) {
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
        for x0 in [704, 1204] {
            p.fill(
                rect_f(x0 as f32, 280.0, (x0 + 13) as f32, 285.0),
                a,
                [250; 3],
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
    }

    pub(crate) fn paint(&self, p: &mut Painter<'_>, frame: u32) {
        let s = &self.spec;
        if frame < s.start || frame > s.end() {
            return;
        }
        let t = frame - s.start;
        let fade_from = s.settle() + s.hold;
        // Box and plate fade in, the name fades out first.
        let alpha = if t < INTRO {
            (t + 1) as f32 / INTRO as f32
        } else {
            1.0
        };
        let name_alpha = if frame >= fade_from {
            (1.0 - (frame - fade_from + 1) as f32 / (s.fade + 1) as f32).max(0.0)
        } else {
            1.0
        };
        let a = Anchor::Center;
        self.paint_box(p, alpha);
        if t < INTRO {
            return;
        }
        let slide_t = (t - INTRO).min(s.slide_frames) as f32 / s.slide_frames.max(1) as f32;
        let offset = s.slide_px * (1.0 - slide_t) * (1.0 - slide_t);
        // The plate slides with the name.
        p.fill(
            rect_f(
                PLATE.x0 as f32 + offset,
                PLATE.y0 as f32,
                PLATE.x1 as f32 + offset,
                PLATE.y1 as f32,
            ),
            a,
            [190, 190, 196],
            0.6 * alpha,
        );
        if name_alpha <= 0.0 {
            return;
        }
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
            name_alpha,
            None,
        );
    }
}

/// An "X ULT READY" notification: white text centred above the plate's
/// place, without box, ticks or plate.
#[derive(Debug, Clone, PartialEq)]
pub struct UltReadySpec {
    /// The ability key shown before "ULT READY".
    pub key: char,
    /// First frame.
    pub start: u32,
    /// Number of frames.
    pub frames: u32,
    /// Baseline of the text, [`ULT_READY_BASELINE`] in the game; tests
    /// move it onto the name's baseline to check the plate gate alone.
    pub baseline: f32,
}

impl UltReadySpec {
    /// A notification at the measured place.
    pub const fn new(key: char, start: u32, frames: u32) -> Self {
        Self {
            key,
            start,
            frames,
            baseline: ULT_READY_BASELINE,
        }
    }
}

/// A notification rendered for one layout.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct UltReady {
    spec: UltReadySpec,
    text: Label,
}

impl UltReady {
    pub(crate) fn new(fonts: &FontSet, layout: &Layout, spec: UltReadySpec) -> Self {
        let text = Label::new(fonts, layout, &format!("{} ULT READY", spec.key), NAME_CAP);
        Self { spec, text }
    }

    pub(crate) fn paint(&self, p: &mut Painter<'_>, frame: u32) {
        let s = &self.spec;
        if frame < s.start || frame - s.start >= s.frames {
            return;
        }
        p.label(
            ContentKind::Notification,
            &self.text,
            (960.0 - self.text.width * 0.5, s.baseline),
            Anchor::Center,
            TEXT_WHITE,
            1.0,
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
        assert_eq!(s.settle(), 133);
        assert_eq!(s.end(), 305);
        // First slide step is 51 px: 270 × (1 − (1 − 1/10)²).
        let first = 270.0 - 270.0 * (9.0f32 / 10.0).powi(2);
        assert!((first - 51.3).abs() < 0.1);
    }
}
