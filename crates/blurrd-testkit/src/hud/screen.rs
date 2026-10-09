//! Timing shared by the screen surfaces (scoreboard, death recap,
//! "Spectating" banner, match-end screens).
//!
//! Measured on real recordings, the scoreboard opens and closes within
//! one frame (it follows the tab key), while panels such as the death
//! recap and the match-end screens can fade and slide. A
//! [`ScreenTiming`] covers both: an optional fade-in during which the
//! surface also slides into place from below, a hold, and an optional
//! fade-out.

use blurrd_core::RectI;

use crate::layout::Anchor;
use crate::paint::Painter;

/// When a screen surface is visible and how it appears.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenTiming {
    /// First frame in which the surface is visible.
    pub start: u32,
    /// Frames of the fade-in (0 = fully visible in the first frame).
    pub fade_in: u32,
    /// Frames the surface stays fully visible.
    pub hold: u32,
    /// Frames of the fade-out (0 = gone after the hold).
    pub fade_out: u32,
    /// Distance in reference pixels the surface slides up during the
    /// fade-in.
    pub slide_px: f32,
}

impl ScreenTiming {
    /// A surface that opens and closes within one frame, like the
    /// scoreboard.
    pub const fn instant(start: u32, hold: u32) -> Self {
        Self {
            start,
            fade_in: 0,
            hold,
            fade_out: 0,
            slide_px: 0.0,
        }
    }

    /// A surface that fades in while sliding up by `slide_px`, holds and
    /// fades out.
    pub const fn animated(
        start: u32,
        fade_in: u32,
        hold: u32,
        fade_out: u32,
        slide_px: f32,
    ) -> Self {
        Self {
            start,
            fade_in,
            hold,
            fade_out,
            slide_px,
        }
    }

    /// The last frame in which the surface is visible.
    pub const fn end(&self) -> u32 {
        self.start + self.fade_in + self.hold + self.fade_out - 1
    }

    /// Opacity and vertical offset (reference pixels, positive = lower)
    /// in `frame`, or `None` when the surface is not visible.
    pub(crate) fn phase(&self, frame: u32) -> Option<(f32, f32)> {
        if frame < self.start || frame > self.end() {
            return None;
        }
        let t = frame - self.start;
        if t < self.fade_in {
            let k = (t + 1) as f32 / (self.fade_in + 1) as f32;
            let offset = self.slide_px * (1.0 - k) * (1.0 - k);
            return Some((k, offset));
        }
        let fade_from = self.fade_in + self.hold;
        if t >= fade_from {
            let k = 1.0 - (t - fade_from + 1) as f32 / (self.fade_out + 1) as f32;
            return Some((k, 0.0));
        }
        Some((1.0, 0.0))
    }
}

/// A one-pixel outline inside `r`.
pub(crate) fn outline(p: &mut Painter<'_>, r: RectI, anchor: Anchor, rgb: [u8; 3], alpha: f32) {
    for edge in [
        RectI { y1: r.y0 + 1, ..r },
        RectI { y0: r.y1 - 1, ..r },
        RectI { x1: r.x0 + 1, ..r },
        RectI { x0: r.x1 - 1, ..r },
    ] {
        p.fill(edge, anchor, rgb, alpha);
    }
}

#[cfg(test)]
mod tests {
    use super::ScreenTiming;

    #[test]
    fn instant_surfaces_have_no_partial_frames() {
        let t = ScreenTiming::instant(10, 5);
        assert_eq!(t.end(), 14);
        assert_eq!(t.phase(9), None);
        assert_eq!(t.phase(10), Some((1.0, 0.0)));
        assert_eq!(t.phase(14), Some((1.0, 0.0)));
        assert_eq!(t.phase(15), None);
    }

    #[test]
    fn animated_surfaces_fade_and_slide() {
        let t = ScreenTiming::animated(0, 3, 4, 3, 40.0);
        assert_eq!(t.end(), 9);
        let (a0, y0) = t.phase(0).unwrap();
        assert!(a0 > 0.0 && a0 < 1.0 && y0 > 0.0);
        let (a2, y2) = t.phase(2).unwrap();
        assert!(a2 > a0 && y2 < y0);
        assert_eq!(t.phase(3), Some((1.0, 0.0)));
        let (a9, _) = t.phase(9).unwrap();
        assert!(a9 > 0.0 && a9 < 1.0);
    }
}
