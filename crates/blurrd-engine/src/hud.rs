//! Where the HUD is in the source frames, and the crops the detectors read.
//!
//! The detectors work in reference pixels of a 1920 × 1080 frame. The
//! [`HudMap`] maps reference rectangles onto the recording with the HUD
//! geometry (each HUD element keeps its distance to the left edge, the
//! right edge or the centre of the HUD); [`RefCrop`] cuts a reference
//! rectangle out of a decoded frame, converts it to RGB and, when the
//! recording is not at the reference scale, resamples it onto the
//! reference grid. Boxes found in reference pixels are mapped back with
//! [`HudMap::source_rect`]. There is no code per resolution: a 1080p
//! recording is simply the case where the mapping is the identity.

use blurrd_core::{HudGeometry, HudLayout, RectI};
use blurrd_detect::screens::{Anchor, ScreenMap};
use blurrd_detect::{ImageError, RgbView};
use blurrd_video::{RgbConverter, RgbImage, VideoError, VideoFrame, YuvRoi};

use crate::resample::{RgbSource, resample};

/// Reference frame size.
pub(crate) const REFERENCE: (u32, u32) = (1920, 1080);

/// Height of the downscaled whole-frame view the screen gates and the
/// scene-cut detector read: a quarter of the reference height.
pub(crate) const QUARTER_HEIGHT: u32 = REFERENCE.1 / 4;

/// The HUD geometry of a `width × height` recording before calibration:
/// a 16:9 HUD scaled with the frame height, anchored to the frame's edges
/// and centre. Exact for every 16:9 recording at the game's default HUD
/// scale; other aspect ratios need the calibrated geometry. Returns the
/// geometry and whether the frame is 16:9.
pub(crate) fn assumed_geometry(width: u32, height: u32) -> (HudGeometry, bool) {
    let scale = height as f32 / REFERENCE.1 as f32;
    let wide = u64::from(width) * 9 == u64::from(height) * 16;
    let geometry = HudGeometry {
        sx: scale,
        sy: scale,
        anchor_left: 0.0,
        anchor_right: width as f32,
        anchor_center: width as f32 / 2.0,
        oy: 0.0,
        layout: if wide {
            HudLayout::Wide16x9
        } else {
            HudLayout::Auto
        },
        ..HudGeometry::default()
    };
    (geometry, wide)
}

/// Reference pixels ↔ source pixels for one recording.
#[derive(Debug, Clone, Copy)]
pub(crate) struct HudMap {
    map: ScreenMap,
    frame: RectI,
}

impl HudMap {
    /// The mapping of `geometry` for a `width × height` recording.
    pub(crate) fn new(geometry: &HudGeometry, width: u32, height: u32) -> Self {
        Self {
            map: ScreenMap::from(geometry),
            frame: RectI {
                x0: 0,
                y0: 0,
                x1: width as i32,
                y1: height as i32,
            },
        }
    }

    /// The screen-surface mapping.
    pub(crate) fn screen_map(&self) -> &ScreenMap {
        &self.map
    }

    /// The frame size.
    pub(crate) fn frame(&self) -> RectI {
        self.frame
    }

    /// The reference rectangle `rect` of an element kept at `anchor`, in
    /// source pixels (rounded outward) and clipped to the frame; `None`
    /// when nothing of it is inside the frame.
    pub(crate) fn source_rect(&self, rect: RectI, anchor: Anchor) -> Option<RectI> {
        self.map.to_source(rect, anchor).intersection(&self.frame)
    }

    /// A reference point of an element kept at `anchor`, in source pixels
    /// (rounded).
    pub(crate) fn point_to_source(&self, x: i32, y: i32, anchor: Anchor) -> (i32, i32) {
        (
            self.map.x(x as f32, anchor).round() as i32,
            self.map.y(y as f32).round() as i32,
        )
    }

    /// Whether reference rectangle `rect` at `anchor` maps onto source
    /// pixels one to one, entirely inside the frame.
    fn is_identity(&self, rect: RectI, anchor: Anchor) -> bool {
        let unscaled = (self.map.sx() - 1.0).abs() < 1e-6 && (self.map.sy() - 1.0).abs() < 1e-6;
        let source = self.map.to_source(rect, anchor);
        unscaled
            && source.width() == rect.width()
            && source.height() == rect.height()
            && source.intersection(&self.frame) == Some(source)
    }

    /// The view pixels per source pixel of a whole-frame view
    /// [`QUARTER_HEIGHT`] pixels high, and its size.
    pub(crate) fn quarter(&self) -> (f32, u32, u32) {
        let height = (self.frame.height()).min(QUARTER_HEIGHT);
        let scale = height as f32 / self.frame.height() as f32;
        let width = ((self.frame.width() as f32 * scale).round() as u32).max(2);
        (scale, width, height)
    }
}

/// A reference rectangle of one frame, cut out of the source and brought
/// to the reference scale. The default is an empty crop (a region that is
/// not analysed).
#[derive(Debug, Clone)]
pub(crate) struct RefCrop {
    /// The reference rectangle.
    reference: RectI,
    /// The source pixels covering it (even-aligned), in RGB.
    native: RgbImage,
    /// Where `native` lies in the source frame.
    native_rect: Option<RectI>,
    /// The resampled reference pixels, unless the mapping is the identity
    /// (then the reference view is a window of `native`).
    resampled: Option<RgbImage>,
}

impl Default for RefCrop {
    fn default() -> Self {
        Self {
            reference: RectI {
                x0: 0,
                y0: 0,
                x1: 0,
                y1: 0,
            },
            native: RgbImage::default(),
            native_rect: None,
            resampled: None,
        }
    }
}

impl RefCrop {
    /// Cuts reference rectangle `reference` of an element kept at `anchor`
    /// out of `frame`. `roi` is a reusable buffer.
    pub(crate) fn extract(
        frame: &VideoFrame,
        map: &HudMap,
        reference: RectI,
        anchor: Anchor,
        converter: &RgbConverter,
        roi: &mut YuvRoi,
    ) -> Result<Self, VideoError> {
        let mut crop = Self {
            reference,
            ..Self::default()
        };
        let identity = map.is_identity(reference, anchor);
        // Two source pixels of margin for the bicubic taps.
        let wanted = map.map.to_source(reference, anchor);
        let wanted = if identity {
            wanted
        } else {
            RectI {
                x0: wanted.x0 - 2,
                y0: wanted.y0 - 2,
                x1: wanted.x1 + 2,
                y1: wanted.y1 + 2,
            }
        };
        let Some(inside) = wanted.intersection(&map.frame) else {
            // Nothing of the element is in the frame: a black crop.
            crop.resampled = Some(black(reference));
            return Ok(crop);
        };
        frame.roi_yuv(inside, roi)?;
        converter.convert(roi, &mut crop.native)?;
        crop.native_rect = roi.rect();
        if !identity {
            let native_rect = roi.rect().unwrap_or(inside);
            let (sx, sy) = (map.map.sx(), map.map.sy());
            let mut out = RgbImage {
                width: reference.width(),
                height: reference.height(),
                data: Vec::new(),
            };
            resample(
                RgbSource {
                    data: &crop.native.data,
                    width: crop.native.width as usize,
                    height: crop.native.height as usize,
                },
                out.width as usize,
                out.height as usize,
                1.0 / sx,
                1.0 / sy,
                |i| map.map.x(reference.x0 as f32 + i as f32 + 0.5, anchor) - native_rect.x0 as f32,
                |j| map.map.y(reference.y0 as f32 + j as f32 + 0.5) - native_rect.y0 as f32,
                &mut out.data,
            );
            crop.resampled = Some(out);
        }
        Ok(crop)
    }

    /// The crop in reference pixels, as the detectors read it.
    pub(crate) fn view(&self) -> Result<RgbView<'_>, ImageError> {
        match (&self.resampled, self.native_rect) {
            (Some(image), _) => RgbView::new(
                &image.data,
                image.width,
                image.height,
                3 * image.width as usize,
            ),
            (None, Some(native)) => {
                let (dx, dy) = (
                    (self.reference.x0 - native.x0) as usize,
                    (self.reference.y0 - native.y0) as usize,
                );
                let stride = 3 * self.native.width as usize;
                RgbView::new(
                    self.native
                        .data
                        .get(dy * stride + 3 * dx..)
                        .unwrap_or_default(),
                    self.reference.width(),
                    self.reference.height(),
                    stride,
                )
            }
            (None, None) => RgbView::new(&[], 0, 0, 0),
        }
    }

    /// The source pixels of `rect` (source coordinates), if the crop holds
    /// them all: what OCR reads at native resolution.
    pub(crate) fn native(&self, rect: RectI) -> Option<blurrd_ocr::RgbView<'_>> {
        let native = self.native_rect?;
        if rect.intersection(&native) != Some(rect) {
            return None;
        }
        let whole =
            blurrd_ocr::RgbView::new(self.native.width, self.native.height, &self.native.data)
                .ok()?;
        whole.crop(RectI {
            x0: rect.x0 - native.x0,
            y0: rect.y0 - native.y0,
            x1: rect.x1 - native.x0,
            y1: rect.y1 - native.y0,
        })
    }
}

/// A black reference crop.
fn black(reference: RectI) -> RgbImage {
    RgbImage {
        width: reference.width(),
        height: reference.height(),
        data: vec![0; 3 * reference.width() as usize * reference.height() as usize],
    }
}

#[cfg(test)]
mod tests {
    use blurrd_core::{HudLayout, RectI};
    use blurrd_detect::screens::Anchor;

    use super::{HudMap, assumed_geometry};

    #[test]
    fn the_assumed_geometry_scales_with_the_height() {
        let (g, wide) = assumed_geometry(1920, 1080);
        assert!(wide && g.layout == HudLayout::Wide16x9);
        assert!((g.sx - 1.0).abs() < 1e-6);
        let (g, wide) = assumed_geometry(2560, 1440);
        assert!(wide);
        assert!((g.sy - 4.0 / 3.0).abs() < 1e-6);
        let (g, wide) = assumed_geometry(1920, 1200);
        assert!(!wide && g.layout == HudLayout::Auto);
    }

    #[test]
    fn maps_reference_boxes_by_anchor() {
        let (g, _) = assumed_geometry(1280, 720);
        let map = HudMap::new(&g, 1280, 720);
        let feed = RectI::new(1782, 140, 1835, 163).unwrap();
        // Right-anchored: 1280 - (1920 - x) * 2/3.
        assert_eq!(
            map.source_rect(feed, Anchor::Right),
            RectI::new(1188, 93, 1224, 109).ok()
        );
        assert_eq!(map.quarter(), (0.375, 480, 270));
        let (g, _) = assumed_geometry(1920, 1080);
        let map = HudMap::new(&g, 1920, 1080);
        assert_eq!(map.source_rect(feed, Anchor::Right), Some(feed));
        assert!(map.is_identity(feed, Anchor::Left));
        assert_eq!(map.quarter(), (0.25, 480, 270));
    }
}
