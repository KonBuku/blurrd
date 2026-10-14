//! Where the HUD sits in a video frame.
//!
//! Detector constants are defined in a 1920 × 1080 reference space;
//! [`HudGeometry`] maps that space onto the source frame. The game scales
//! its HUD with the image height and keeps every element at the screen edge
//! or centre it is anchored to ([`HudAnchor`]), so one mapping per layout
//! covers every resolution: [`HudGeometry::for_layout`] builds it, and the
//! detector's calibration confirms the layout from the frames.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::geometry::RectI;
use crate::region::Region;

/// Width of the reference HUD space in pixels.
pub const REFERENCE_WIDTH: u32 = 1920;
/// Height of the reference HUD space in pixels.
pub const REFERENCE_HEIGHT: u32 = 1080;

/// Which position a HUD element keeps when the frame's width changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HudAnchor {
    /// The left edge of the game image (voice and text chat).
    Left,
    /// The right edge of the game image (kill feed).
    Right,
    /// The horizontal centre of the game image (banners, screens).
    Center,
}

impl HudAnchor {
    /// The reference x of the anchor itself.
    const fn reference_x(self) -> f32 {
        match self {
            Self::Left => 0.0,
            Self::Right => REFERENCE_WIDTH as f32,
            Self::Center => REFERENCE_WIDTH as f32 * 0.5,
        }
    }
}

/// How the game's HUD is laid out in the recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum HudLayout {
    /// Detect the layout from the frames.
    #[default]
    Auto,
    /// 16:9 game resolution in a 16:9 recording.
    Wide16x9,
    /// Native 4:3 recording (for example 1280 × 960).
    Native4x3,
    /// 4:3 game resolution stretched to fill a 16:9 recording.
    Stretched4x3,
    /// 16:10 recording (for example 1920 × 1200).
    Wide16x10,
    /// 4:3 image with black bars of `bar_px` source pixels on both sides.
    Pillarbox {
        /// Width of each black bar in source pixels.
        bar_px: u16,
    },
    /// HUD scaled by the game's HUD scale setting.
    Custom {
        /// HUD scale in percent.
        scale_pct: u16,
    },
}

/// Calibration confidence per region, from 0.0 to 1.0.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(transparent)]
pub struct RegionCalibration(pub BTreeMap<Region, f32>);

/// Mapping of the 1920 × 1080 reference HUD onto the source frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct HudGeometry {
    /// Horizontal scale from reference to source pixels.
    pub sx: f32,
    /// Vertical scale from reference to source pixels.
    pub sy: f32,
    /// Source x of the HUD's left edge (left-anchored elements).
    pub anchor_left: f32,
    /// Source x of the HUD's right edge (right-anchored elements).
    pub anchor_right: f32,
    /// Source x of the HUD's centre (centred elements).
    pub anchor_center: f32,
    /// Vertical offset in source pixels.
    pub oy: f32,
    /// The detected or chosen layout.
    pub layout: HudLayout,
    /// Calibration confidence per region.
    pub per_region: RegionCalibration,
    /// Overall calibration confidence, from 0.0 to 1.0.
    pub confidence: f32,
}

impl Default for HudGeometry {
    /// The identity mapping for a 1920 × 1080 recording, not yet calibrated
    /// (confidence 0).
    fn default() -> Self {
        Self {
            sx: 1.0,
            sy: 1.0,
            anchor_left: 0.0,
            anchor_right: 1920.0,
            anchor_center: 960.0,
            oy: 0.0,
            layout: HudLayout::Auto,
            per_region: RegionCalibration::default(),
            confidence: 0.0,
        }
    }
}

impl HudGeometry {
    /// The mapping of a recording `width` × `height` pixels with `layout`,
    /// not yet confirmed by calibration (confidence 0).
    ///
    /// The HUD scales with the image height (`sy = height / 1080`) and its
    /// elements keep their anchors at the image edges: a stretched 4:3 game
    /// also widens it by the frame's aspect over 4:3, a pillarbox moves the
    /// edges in by the bar width. [`HudLayout::Auto`] takes the layout from
    /// the frame's aspect ratio. [`HudLayout::Custom`] scales the HUD by its
    /// percentage around the anchors and the vertical centre.
    pub fn for_layout(width: u32, height: u32, layout: HudLayout) -> Self {
        let (w, h) = (width as f32, height as f32);
        let sy = h / REFERENCE_HEIGHT as f32;
        let layout = match layout {
            HudLayout::Auto => layout_of_aspect(width, height),
            other => other,
        };
        let (sx, sy, bar, oy) = match layout {
            HudLayout::Stretched4x3 => (sy * (w / h) / (4.0 / 3.0), sy, 0.0, 0.0),
            HudLayout::Pillarbox { bar_px } => (sy, sy, f32::from(bar_px), 0.0),
            HudLayout::Custom { scale_pct } => {
                let s = sy * f32::from(scale_pct) / 100.0;
                (s, s, 0.0, 0.5 * (h - REFERENCE_HEIGHT as f32 * s))
            }
            _ => (sy, sy, 0.0, 0.0),
        };
        Self {
            sx,
            sy,
            anchor_left: bar,
            anchor_right: w - bar,
            anchor_center: 0.5 * w,
            oy,
            layout,
            per_region: RegionCalibration::default(),
            confidence: 0.0,
        }
    }

    /// Whether this is the identity mapping of a 1920 × 1080 recording.
    #[allow(
        clippy::float_cmp,
        reason = "the identity mapping holds exact integer values"
    )]
    pub fn is_reference(&self) -> bool {
        self.sx == 1.0
            && self.sy == 1.0
            && self.anchor_left == 0.0
            && self.anchor_right == REFERENCE_WIDTH as f32
            && self.anchor_center == REFERENCE_WIDTH as f32 * 0.5
            && self.oy == 0.0
    }

    /// The source x of an anchor.
    const fn anchor_x(&self, anchor: HudAnchor) -> f32 {
        match anchor {
            HudAnchor::Left => self.anchor_left,
            HudAnchor::Right => self.anchor_right,
            HudAnchor::Center => self.anchor_center,
        }
    }

    /// Source x of reference x `x` for an element with `anchor`.
    pub fn source_x(&self, x: f32, anchor: HudAnchor) -> f32 {
        self.anchor_x(anchor) + (x - anchor.reference_x()) * self.sx
    }

    /// Source y of reference y `y`.
    pub fn source_y(&self, y: f32) -> f32 {
        self.oy + y * self.sy
    }

    /// Reference x of source x `x` for an element with `anchor`.
    pub fn reference_x(&self, x: f32, anchor: HudAnchor) -> f32 {
        anchor.reference_x() + (x - self.anchor_x(anchor)) / self.sx
    }

    /// Reference y of source y `y`.
    pub fn reference_y(&self, y: f32) -> f32 {
        (y - self.oy) / self.sy
    }

    /// A reference rectangle in source pixels, rounded outward (at least
    /// one pixel wide and tall).
    pub fn to_source(&self, r: RectI, anchor: HudAnchor) -> RectI {
        outward(
            self.source_x(r.x0 as f32, anchor),
            self.source_y(r.y0 as f32),
            self.source_x(r.x1 as f32, anchor),
            self.source_y(r.y1 as f32),
        )
    }

    /// A source rectangle in reference pixels, rounded outward (at least
    /// one pixel wide and tall).
    pub fn to_reference(&self, r: RectI, anchor: HudAnchor) -> RectI {
        outward(
            self.reference_x(r.x0 as f32, anchor),
            self.reference_y(r.y0 as f32),
            self.reference_x(r.x1 as f32, anchor),
            self.reference_y(r.y1 as f32),
        )
    }
}

/// The layout a frame's aspect ratio suggests: 4:3, 16:10, or 16:9 for
/// every other aspect (mapped by height).
fn layout_of_aspect(width: u32, height: u32) -> HudLayout {
    let aspect = width as f32 / height.max(1) as f32;
    if (aspect - 4.0 / 3.0).abs() < 0.02 {
        HudLayout::Native4x3
    } else if (aspect - 1.6).abs() < 0.02 {
        HudLayout::Wide16x10
    } else {
        HudLayout::Wide16x9
    }
}

/// The smallest integer rectangle around `x0..x1` × `y0..y1`, at least one
/// pixel wide and tall. The edges move in by a hundredth of a pixel before
/// rounding, so float noise in an exact mapping does not add a pixel.
fn outward(x0: f32, y0: f32, x1: f32, y1: f32) -> RectI {
    const EPS: f32 = 0.01;
    let (x0, y0) = ((x0 + EPS).floor() as i32, (y0 + EPS).floor() as i32);
    let (x1, y1) = ((x1 - EPS).ceil() as i32, (y1 - EPS).ceil() as i32);
    RectI {
        x0,
        y0,
        x1: x1.max(x0 + 1),
        y1: y1.max(y0 + 1),
    }
}

#[cfg(test)]
mod tests {
    use super::{HudAnchor, HudGeometry, HudLayout};
    use crate::geometry::RectI;

    const ANCHORS: [HudAnchor; 3] = [HudAnchor::Left, HudAnchor::Right, HudAnchor::Center];

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn a_1080p_recording_is_the_identity() {
        let g = HudGeometry::for_layout(1920, 1080, HudLayout::Auto);
        assert_eq!(g.layout, HudLayout::Wide16x9);
        assert!(g.is_reference());
        let r = RectI::new(660, 80, 1920, 412).unwrap();
        for anchor in ANCHORS {
            assert_eq!(g.to_source(r, anchor), r);
            assert_eq!(g.to_reference(r, anchor), r);
        }
        let default = HudGeometry {
            layout: HudLayout::Wide16x9,
            ..HudGeometry::default()
        };
        assert_eq!(g, default);
    }

    #[test]
    fn scales_with_height_and_keeps_the_anchors() {
        let qhd = HudGeometry::for_layout(2560, 1440, HudLayout::Auto);
        assert!(close(qhd.sx, 4.0 / 3.0) && close(qhd.sy, 4.0 / 3.0));
        assert!(close(
            qhd.source_x(1900.0, HudAnchor::Right),
            2560.0 - 20.0 * 4.0 / 3.0
        ));
        let native = HudGeometry::for_layout(1280, 960, HudLayout::Auto);
        assert_eq!(native.layout, HudLayout::Native4x3);
        let s = 960.0 / 1080.0;
        assert!(close(
            native.source_x(1900.0, HudAnchor::Right),
            1280.0 - 20.0 * s
        ));
        assert!(close(native.source_x(40.0, HudAnchor::Left), 40.0 * s));
        assert!(close(native.source_x(960.0, HudAnchor::Center), 640.0));
        let tall = HudGeometry::for_layout(1920, 1200, HudLayout::Auto);
        assert_eq!(tall.layout, HudLayout::Wide16x10);
        assert!(close(tall.source_y(1080.0), 1200.0));
        assert!(!tall.is_reference());
    }

    #[test]
    fn maps_pillarbox_stretched_and_custom_scale() {
        let p = HudGeometry::for_layout(1920, 1080, HudLayout::Pillarbox { bar_px: 240 });
        assert!(close(p.source_x(0.0, HudAnchor::Left), 240.0));
        assert!(close(p.source_x(1920.0, HudAnchor::Right), 1680.0));
        assert!(close(p.source_x(960.0, HudAnchor::Center), 960.0));
        let s = HudGeometry::for_layout(1920, 1080, HudLayout::Stretched4x3);
        assert!(close(s.sx, 4.0 / 3.0) && close(s.sy, 1.0));
        assert!(close(
            s.source_x(1900.0, HudAnchor::Right),
            1920.0 - 20.0 * 4.0 / 3.0
        ));
        let c = HudGeometry::for_layout(1920, 1080, HudLayout::Custom { scale_pct: 80 });
        assert!(close(c.sy, 0.8) && close(c.source_y(540.0), 540.0));
    }

    #[test]
    fn source_and_reference_round_trip() {
        let layouts = [
            (1280, 720, HudLayout::Auto),
            (3840, 2160, HudLayout::Auto),
            (1280, 960, HudLayout::Auto),
            (1920, 1080, HudLayout::Pillarbox { bar_px: 240 }),
            (1920, 1080, HudLayout::Stretched4x3),
        ];
        let r = RectI::new(1250, 96, 1838, 129).unwrap();
        for (w, h, layout) in layouts {
            let g = HudGeometry::for_layout(w, h, layout);
            for anchor in ANCHORS {
                let back = g.to_reference(g.to_source(r, anchor), anchor);
                // Rounded outward twice: at most two pixels per edge.
                assert!((r.x0 - 2..=r.x0).contains(&back.x0), "{w}x{h} {back:?}");
                assert!((r.x1..=r.x1 + 2).contains(&back.x1), "{w}x{h} {back:?}");
                assert!((r.y0 - 2..=r.y0).contains(&back.y0), "{w}x{h} {back:?}");
                assert!((r.y1..=r.y1 + 2).contains(&back.y1), "{w}x{h} {back:?}");
            }
        }
    }
}
