//! Frame sizes and HUD layouts: how the 1920 × 1080 reference HUD maps
//! onto a recording.
//!
//! The game lays out its HUD on a canvas that scales with the frame height
//! and anchors each element to a screen edge or the centre (architecture
//! plan 7.3, port spec section 9). A [`Layout`] is that mapping with known
//! parameters, so fixtures in every layout come with their true
//! [`HudGeometry`].

use blurrd_core::{HudGeometry, HudLayout, RectI};

/// The reference HUD width.
pub const REF_WIDTH: f32 = 1920.0;
/// The reference HUD height.
pub const REF_HEIGHT: f32 = 1080.0;

/// Which screen position a HUD element keeps when the frame changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Anchor {
    /// Left edge (voice and text chat).
    Left,
    /// Right edge (kill feed).
    Right,
    /// Horizontal centre (banners).
    Center,
}

/// A frame size together with the mapping of the reference HUD onto it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// The layout kind.
    pub kind: HudLayout,
    /// Horizontal scale from reference to source pixels.
    pub sx: f32,
    /// Vertical scale from reference to source pixels.
    pub sy: f32,
    /// Source x of the reference left edge.
    pub left: f32,
    /// Source x of the reference right edge.
    pub right: f32,
    /// Source y of the reference top edge.
    pub top: f32,
    /// The part of the frame that shows the game (the frame minus black
    /// bars).
    pub image: RectI,
}

impl Layout {
    /// A 16:9 recording of a 16:9 game at any resolution (1280 × 720,
    /// 1920 × 1080, 2560 × 1440, 3840 × 2160).
    pub fn wide(width: u32, height: u32) -> Self {
        let sy = height as f32 / REF_HEIGHT;
        Self::anchored(width, height, HudLayout::Wide16x9, sy, sy, 0)
    }

    /// The reference layout, 1920 × 1080.
    pub fn reference() -> Self {
        Self::wide(1920, 1080)
    }

    /// A native 4:3 recording such as 1280 × 960: the HUD keeps its height
    /// scale and its edge anchors, so right-anchored elements move left.
    pub fn native_4x3(width: u32, height: u32) -> Self {
        let sy = height as f32 / REF_HEIGHT;
        Self::anchored(width, height, HudLayout::Native4x3, sy, sy, 0)
    }

    /// A 16:10 recording such as 1920 × 1200.
    pub fn wide_16x10(width: u32, height: u32) -> Self {
        let sy = height as f32 / REF_HEIGHT;
        Self::anchored(width, height, HudLayout::Wide16x10, sy, sy, 0)
    }

    /// A 4:3 game stretched to fill a 16:9 frame: horizontal scale
    /// `sy × (W / H) / (4 / 3)`.
    pub fn stretched_4x3(width: u32, height: u32) -> Self {
        let sy = height as f32 / REF_HEIGHT;
        let sx = sy * (width as f32 / height as f32) / (4.0 / 3.0);
        Self::anchored(width, height, HudLayout::Stretched4x3, sx, sy, 0)
    }

    /// A 4:3 image centred in a 16:9 frame with black bars left and right.
    pub fn pillarbox(width: u32, height: u32) -> Self {
        let sy = height as f32 / REF_HEIGHT;
        let image_w = (height as f32 * 4.0 / 3.0).round() as u32;
        let bar = width.saturating_sub(image_w) / 2;
        let kind = HudLayout::Pillarbox {
            bar_px: u16::try_from(bar).unwrap_or(u16::MAX),
        };
        Self::anchored(width, height, kind, sy, sy, bar)
    }

    /// A 16:9 game image scaled to fill a frame of another aspect ratio,
    /// such as a 1080p recording saved as 1440 × 1080: the whole HUD
    /// narrows with the image (horizontal scale `W / 1920`).
    pub fn scaled_16x9(width: u32, height: u32) -> Self {
        let sy = height as f32 / REF_HEIGHT;
        let sx = width as f32 / REF_WIDTH;
        Self::anchored(
            width,
            height,
            HudLayout::Scaled16x9 { bar_px: 0 },
            sx,
            sy,
            0,
        )
    }

    fn anchored(width: u32, height: u32, kind: HudLayout, sx: f32, sy: f32, bar: u32) -> Self {
        let (w, h) = (width as i32, height as i32);
        let bar_i = bar as i32;
        Self {
            width,
            height,
            kind,
            sx,
            sy,
            left: bar as f32,
            right: (width - bar) as f32,
            top: 0.0,
            image: RectI {
                x0: bar_i,
                y0: 0,
                x1: w - bar_i,
                y1: h,
            },
        }
    }

    /// The layouts the fixture matrix covers: 16:9 at four resolutions,
    /// native 4:3, 4:3 pillarboxed, 16:10 and 4:3 stretched.
    pub fn matrix() -> [Self; 8] {
        [
            Self::wide(1280, 720),
            Self::reference(),
            Self::wide(2560, 1440),
            Self::wide(3840, 2160),
            Self::native_4x3(1280, 960),
            Self::pillarbox(1920, 1080),
            Self::wide_16x10(1920, 1200),
            Self::stretched_4x3(1920, 1080),
        ]
    }

    /// Source x of the anchor itself.
    fn anchor_x(&self, anchor: Anchor) -> (f32, f32) {
        match anchor {
            Anchor::Left => (self.left, 0.0),
            Anchor::Right => (self.right, REF_WIDTH),
            Anchor::Center => (f32::midpoint(self.left, self.right), REF_WIDTH * 0.5),
        }
    }

    /// Source x of reference x `x` for an element with `anchor`.
    pub fn x(&self, x: f32, anchor: Anchor) -> f32 {
        let (src, reference) = self.anchor_x(anchor);
        src + (x - reference) * self.sx
    }

    /// Source y of reference y `y`.
    pub fn y(&self, y: f32) -> f32 {
        self.top + y * self.sy
    }

    /// A reference rectangle mapped to source pixels, rounded outward.
    pub fn rect(&self, r: RectI, anchor: Anchor) -> RectI {
        let x0 = self.x(r.x0 as f32, anchor).floor() as i32;
        let x1 = self.x(r.x1 as f32, anchor).ceil() as i32;
        let y0 = self.y(r.y0 as f32).floor() as i32;
        let y1 = self.y(r.y1 as f32).ceil() as i32;
        RectI {
            x0,
            y0,
            x1: x1.max(x0 + 1),
            y1: y1.max(y0 + 1),
        }
    }

    /// A length of `px` reference pixels in source pixels along x and
    /// along y, each rounded up and at least 1 (for halos and shadow
    /// offsets). The axes differ in stretched layouts: a 4:3 image
    /// stretched to 16:9 widens the HUD but keeps its height, so its text
    /// shadow and halo stay as tall as at 1080p.
    pub fn len_px(&self, px: f32) -> (u32, u32) {
        let len = |scale: f32| (px * scale).ceil().max(1.0) as u32;
        (len(self.sx), len(self.sy))
    }

    /// The true HUD geometry of this layout.
    pub fn geometry(&self) -> HudGeometry {
        HudGeometry {
            sx: self.sx,
            sy: self.sy,
            anchor_left: self.left,
            anchor_right: self.right,
            anchor_center: f32::midpoint(self.left, self.right),
            oy: self.top,
            layout: self.kind,
            confidence: 1.0,
            ..HudGeometry::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use blurrd_core::{HudLayout, RectI};

    use super::{Anchor, Layout};

    #[test]
    fn reference_layout_is_the_identity() {
        let l = Layout::reference();
        let r = RectI::new(1250, 96, 1900, 129).unwrap();
        for anchor in [Anchor::Left, Anchor::Right, Anchor::Center] {
            assert_eq!(l.rect(r, anchor), r);
        }
        assert_eq!(l.geometry().anchor_right, 1920.0);
    }

    #[test]
    fn scales_with_height_and_keeps_anchors() {
        let l = Layout::wide(2560, 1440);
        assert!((l.x(1900.0, Anchor::Right) - 2533.333).abs() < 0.01);
        let n = Layout::native_4x3(1280, 960);
        // Right-anchored: 20 reference px from the right edge.
        assert!((n.x(1900.0, Anchor::Right) - (1280.0 - 20.0 * 960.0 / 1080.0)).abs() < 0.01);
        assert!((n.x(40.0, Anchor::Left) - 40.0 * 960.0 / 1080.0).abs() < 0.01);
    }

    #[test]
    fn pillarbox_and_stretch() {
        let p = Layout::pillarbox(1920, 1080);
        assert_eq!(p.kind, HudLayout::Pillarbox { bar_px: 240 });
        assert_eq!(p.image, RectI::new(240, 0, 1680, 1080).unwrap());
        assert!((p.x(0.0, Anchor::Left) - 240.0).abs() < f32::EPSILON);
        let s = Layout::stretched_4x3(1920, 1080);
        assert!((s.sx - 4.0 / 3.0).abs() < 1e-6 && (s.sy - 1.0).abs() < f32::EPSILON);
        assert!((s.x(960.0, Anchor::Center) - 960.0).abs() < f32::EPSILON);
    }

    #[test]
    fn matrix_has_distinct_frames() {
        let m = Layout::matrix();
        for (i, a) in m.iter().enumerate() {
            for b in &m[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }
}
