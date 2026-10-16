//! Where the HUD is in the source frames, and the crops the detectors read.
//!
//! The detectors work in reference pixels of a 1920 × 1080 frame. The
//! [`HudMap`] maps reference rectangles onto the recording with the HUD
//! geometry (each HUD element keeps its distance to the left edge, the
//! right edge or the centre of the HUD). Each frame is first brought to
//! the reference scale as a whole (the *reference frame*: a 16:9
//! recording becomes 1920 × 1080, other aspect ratios keep theirs) the
//! way the detectors were tuned and validated (see `scaler`); the crops
//! the detectors read are then plain windows of it ([`Crop`]), black
//! where an element lies outside the frame. Boxes found in reference
//! pixels are mapped back with [`HudMap::source_rect`]. There is no code
//! per resolution: a 1080p recording is simply the case where the
//! reference frame is the frame.

use blurrd_core::{HudGeometry, RectI};
use blurrd_detect::screens::{Anchor, ScreenMap};
use blurrd_detect::{ImageError, RgbView};
use blurrd_video::RgbImage;

/// Reference pixels per pixel of the downscaled whole-frame view that the
/// screen gates and the scene-cut detector read (4 × 4 means).
pub(crate) const QUARTER: u32 = 4;

/// Reference pixels ↔ source pixels for one recording.
#[derive(Debug, Clone, Copy)]
pub(crate) struct HudMap {
    map: ScreenMap,
    frame: RectI,
    /// The size of the frame at the reference scale.
    reference_size: (u32, u32),
    /// Reference pixels ↔ pixels of the reference frame (an offset per
    /// anchor, no scaling).
    reference: ScreenMap,
}

impl HudMap {
    /// The mapping of `geometry` for a `width × height` recording.
    pub(crate) fn new(geometry: &HudGeometry, width: u32, height: u32) -> Self {
        let (w, h) = even_size(width, height);
        let even = |v: f32| ((v / 2.0).round().max(1.0) as u32) * 2;
        let reference_size = (even(w as f32 / geometry.sx), even(h as f32 / geometry.sy));
        let (fx, fy) = (
            w as f32 / reference_size.0 as f32,
            h as f32 / reference_size.1 as f32,
        );
        let reference = ScreenMap::from(&HudGeometry {
            sx: 1.0,
            sy: 1.0,
            anchor_left: geometry.anchor_left / fx,
            anchor_right: geometry.anchor_right / fx,
            anchor_center: geometry.anchor_center / fx,
            oy: geometry.oy / fy,
            ..HudGeometry::default()
        });
        Self {
            map: ScreenMap::from(geometry),
            frame: RectI {
                x0: 0,
                y0: 0,
                x1: width as i32,
                y1: height as i32,
            },
            reference_size,
            reference,
        }
    }

    /// The screen-surface mapping onto the source frame.
    pub(crate) fn screen_map(&self) -> &ScreenMap {
        &self.map
    }

    /// The size of the reference frame.
    pub(crate) fn reference_size(&self) -> (u32, u32) {
        self.reference_size
    }

    /// The mapping of reference pixels onto the reference frame.
    pub(crate) fn reference_map(&self) -> &ScreenMap {
        &self.reference
    }

    /// Whether the reference frame is the (even-sized) frame itself.
    pub(crate) fn is_identity(&self) -> bool {
        self.reference_size == even_size(self.frame.width(), self.frame.height())
    }

    /// Where reference rectangle `rect` of an element kept at `anchor`
    /// lies in the reference frame: the same size, moved by the anchor's
    /// whole-pixel offset (it may reach outside the reference frame).
    pub(crate) fn reference_rect(&self, rect: RectI, anchor: Anchor) -> RectI {
        let dx = self.reference.x(rect.x0 as f32, anchor).round() as i32 - rect.x0;
        let dy = self.reference.y(rect.y0 as f32).round() as i32 - rect.y0;
        RectI {
            x0: rect.x0 + dx,
            y0: rect.y0 + dy,
            x1: rect.x1 + dx,
            y1: rect.y1 + dy,
        }
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
}

/// The size 4:2:0 frames are read at: an odd last row or column is left
/// out.
fn even_size(width: u32, height: u32) -> (u32, u32) {
    ((width & !1).max(2), (height & !1).max(2))
}

/// A rectangle of RGB pixels copied out of a bigger image, black where it
/// reaches outside that image. The default is an empty crop (a region
/// that is not analysed).
#[derive(Debug, Clone, Default)]
pub(crate) struct Crop {
    /// The pixels.
    image: RgbImage,
    /// Where the crop lies in the image it came from.
    rect: Option<RectI>,
}

impl Crop {
    /// A black crop of `rect`, to be filled with [`Crop::fill`].
    pub(crate) fn black(rect: RectI) -> Self {
        Self {
            image: RgbImage {
                width: rect.width(),
                height: rect.height(),
                data: vec![0u8; 3 * rect.width() as usize * rect.height() as usize],
            },
            rect: Some(rect),
        }
    }

    /// Where the crop lies in the image it came from; `None` for an
    /// empty crop.
    pub(crate) fn rect(&self) -> Option<RectI> {
        self.rect
    }

    /// Copies the part of the crop's rectangle that `source` holds, where
    /// `source` shows the image the crop comes from from `origin` (its top
    /// left pixel) on.
    pub(crate) fn fill(&mut self, source: &RgbImage, (x, y): (i32, i32)) {
        let Some(rect) = self.rect else {
            return;
        };
        let bounds = RectI {
            x0: x,
            y0: y,
            x1: x + source.width as i32,
            y1: y + source.height as i32,
        };
        let Some(inside) = rect.intersection(&bounds) else {
            return;
        };
        let (stride, w) = (3 * source.width as usize, 3 * rect.width() as usize);
        let len = 3 * inside.width() as usize;
        let (from_x, to_x) = (
            3 * (inside.x0 - x) as usize,
            3 * (inside.x0 - rect.x0) as usize,
        );
        for row in inside.y0..inside.y1 {
            let from = (row - y) as usize * stride + from_x;
            let to = (row - rect.y0) as usize * w + to_x;
            self.image.data[to..to + len].copy_from_slice(&source.data[from..from + len]);
        }
    }

    /// Takes `image`, which shows `rect` of the image it came from.
    pub(crate) fn from_image(image: RgbImage, rect: RectI) -> Self {
        Self {
            image,
            rect: Some(rect),
        }
    }

    /// The pixels, as the detectors read them.
    pub(crate) fn view(&self) -> Result<RgbView<'_>, ImageError> {
        rgb_view(&self.image)
    }

    /// The pixels of `rect` (coordinates of the image the crop came from),
    /// if the crop holds them all: what OCR reads.
    pub(crate) fn ocr_view(&self, rect: RectI) -> Option<blurrd_ocr::RgbView<'_>> {
        let own = self.rect?;
        if rect.intersection(&own) != Some(rect) {
            return None;
        }
        let whole =
            blurrd_ocr::RgbView::new(self.image.width, self.image.height, &self.image.data).ok()?;
        whole.crop(RectI {
            x0: rect.x0 - own.x0,
            y0: rect.y0 - own.y0,
            x1: rect.x1 - own.x0,
            y1: rect.y1 - own.y0,
        })
    }
}

/// A detector view of `image`.
pub(crate) fn rgb_view(image: &RgbImage) -> Result<RgbView<'_>, ImageError> {
    RgbView::new(
        &image.data,
        image.width,
        image.height,
        3 * image.width as usize,
    )
}

/// The 4 × 4 means of `image` into `out` (each channel rounded,
/// `(sum + 8) / 16`): the whole-frame view the screen gates and the
/// scene-cut detector read. Right and bottom remainders narrower than
/// 4 pixels are left out.
pub(crate) fn quarter(image: &RgbImage, out: &mut RgbImage) {
    let q = QUARTER as usize;
    let (w, h) = (image.width as usize, image.height as usize);
    let (qw, qh) = (w / q, h / q);
    out.width = qw as u32;
    out.height = qh as u32;
    out.data.clear();
    out.data.resize(3 * qw * qh, 0);
    let stride = 3 * w;
    // Column sums of 4 rows first (4 × 255 fits u16), then 4 columns.
    let mut sums = vec![0u16; 3 * q * qw];
    for (qy, line) in out.data.chunks_exact_mut(3 * qw).enumerate() {
        sums.fill(0);
        for y in qy * q..(qy + 1) * q {
            let row = &image.data[y * stride..y * stride + sums.len()];
            for (sum, &v) in sums.iter_mut().zip(row) {
                *sum += u16::from(v);
            }
        }
        for (px, block) in line
            .as_chunks_mut::<3>()
            .0
            .iter_mut()
            .zip(sums.chunks_exact(3 * q))
        {
            for (c, value) in px.iter_mut().enumerate() {
                let total: u32 = (0..q).map(|k| u32::from(block[3 * k + c])).sum();
                *value = ((total + 8) / 16) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use blurrd_core::{HudGeometry, HudLayout, RectI};
    use blurrd_detect::screens::Anchor;
    use blurrd_video::RgbImage;

    use super::{Crop, HudMap, quarter};

    #[test]
    fn maps_reference_boxes_by_anchor() {
        let g = HudGeometry::for_layout(1280, 720, HudLayout::Auto);
        let map = HudMap::new(&g, 1280, 720);
        let feed = RectI::new(1782, 140, 1835, 163).unwrap();
        // Right-anchored: 1280 - (1920 - x) * 2/3.
        assert_eq!(
            map.source_rect(feed, Anchor::Right),
            RectI::new(1188, 93, 1224, 109).ok()
        );
        assert_eq!(map.reference_size(), (1920, 1080));
        assert!(!map.is_identity());
        assert_eq!(map.reference_rect(feed, Anchor::Right), feed);
        let g = HudGeometry::for_layout(1920, 1080, HudLayout::Auto);
        let map = HudMap::new(&g, 1920, 1080);
        assert_eq!(map.source_rect(feed, Anchor::Right), Some(feed));
        assert!(map.is_identity());
    }

    #[test]
    fn other_aspect_ratios_keep_their_width() {
        // 21:9 at 1080 lines: the reference frame is the frame, and
        // right-anchored elements move with the right edge.
        let g = HudGeometry::for_layout(2560, 1080, HudLayout::Auto);
        let map = HudMap::new(&g, 2560, 1080);
        assert!(map.is_identity());
        let feed = RectI::new(660, 80, 1920, 412).unwrap();
        assert_eq!(
            map.reference_rect(feed, Anchor::Right),
            RectI::new(1300, 80, 2560, 412).unwrap()
        );
        // Native 4:3 at 960 lines becomes 1440 × 1080.
        let g = HudGeometry::for_layout(1280, 960, HudLayout::Auto);
        let map = HudMap::new(&g, 1280, 960);
        assert_eq!(map.reference_size(), (1440, 1080));
        assert_eq!(
            map.reference_rect(feed, Anchor::Right),
            RectI::new(180, 80, 1440, 412).unwrap()
        );
    }

    #[test]
    fn crops_are_black_outside_the_image() {
        let source = RgbImage {
            width: 4,
            height: 2,
            data: (1..=24).collect(),
        };
        let mut crop = Crop::black(RectI::new(2, -1, 6, 1).unwrap());
        crop.fill(&source, (0, 0));
        let view = crop.view().unwrap();
        assert_eq!((view.width(), view.height()), (4, 2));
        assert_eq!(view.row(0), &[0; 12]);
        assert_eq!(view.row(1), &[7, 8, 9, 10, 11, 12, 0, 0, 0, 0, 0, 0]);

        // A part of the image that starts elsewhere fills the same pixels.
        let part = RgbImage {
            width: 2,
            height: 1,
            data: (7..=12).collect(),
        };
        let mut from_part = Crop::black(RectI::new(2, -1, 6, 1).unwrap());
        from_part.fill(&part, (2, 0));
        assert_eq!(from_part.image, crop.image);
    }

    #[test]
    fn the_quarter_view_rounds_4x4_means() {
        let mut data = vec![0u8; 3 * 8 * 4];
        data[0] = 8; // one pixel of 8 in the first block: (8 + 8) / 16 = 1
        data[3 * 4] = 7; // 7 in the second block rounds down to 0
        let image = RgbImage {
            width: 8,
            height: 4,
            data,
        };
        let mut out = RgbImage::default();
        quarter(&image, &mut out);
        assert_eq!((out.width, out.height), (2, 1));
        assert_eq!(out.data, [1, 0, 0, 0, 0, 0]);
    }
}
