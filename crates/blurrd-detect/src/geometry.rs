//! Reading the HUD of any layout in reference pixels.
//!
//! The HUD detectors work on crops of a 1920 × 1080 reference frame
//! ([`FEED_ROI`](crate::killfeed::FEED_ROI),
//! [`LEFT_PANEL_ROI`](crate::chat::LEFT_PANEL_ROI)). A [`HudSampler`]
//! cuts such a crop out of a frame of any resolution and layout: every
//! reference pixel is mapped onto the source through the video's
//! [`HudGeometry`] for the anchor of the elements in the crop, and the
//! source is resampled there. Detections come back in reference pixels and
//! [`HudSampler::to_source`] maps them onto the frame. There is no code
//! per resolution: a 1080p recording is the identity mapping and gets the
//! source pixels unchanged, every other layout the same resampling.
//!
//! The resampling is separable: upscaling (720p, 900p) interpolates with
//! the cubic convolution kernel (`a = −0.5`), as a bicubic video scaler
//! does; downscaling (1440p, 4K) averages with a triangle kernel widened by
//! the scale, so that every source pixel contributes (no aliasing of thin
//! strokes) and the HUD's hard edges do not ring.

use blurrd_core::{HudAnchor, HudGeometry, RectI};

use crate::image::RgbView;

/// Fixed-point precision of the filter weights (bits).
const WEIGHT_BITS: u32 = 14;
/// The sum of one output pixel's weights.
const WEIGHT_ONE: i32 = 1 << WEIGHT_BITS;
/// The cubic convolution parameter of the kernel.
const CUBIC_A: f32 = -0.5;
/// Source pixels added around a box mapped back from reference pixels
/// ([`HudSampler::to_source`]).
const SOURCE_MARGIN: i32 = 1;

/// The cubic convolution kernel (Keys) at distance `t`.
fn cubic(t: f32) -> f32 {
    let t = t.abs();
    if t < 1.0 {
        ((CUBIC_A + 2.0) * t - (CUBIC_A + 3.0)) * t * t + 1.0
    } else if t < 2.0 {
        (((t - 5.0) * t + 8.0) * t - 4.0) * CUBIC_A
    } else {
        0.0
    }
}

/// The resampling filter of one axis: for every output pixel, the first
/// source index and [`Filter::taps`] fixed-point weights (edge pixels
/// repeated beyond the source).
#[derive(Debug, Clone, Default, PartialEq)]
struct Filter {
    taps: usize,
    starts: Vec<usize>,
    weights: Vec<i32>,
}

impl Filter {
    /// The filter for `len` output pixels whose centres lie at source
    /// positions `position(i)` (in pixels, pixel centres at `k + 0.5`),
    /// with `scale` source pixels per output pixel, over a source of
    /// `source_len` pixels.
    fn new(len: usize, scale: f32, source_len: usize, position: impl Fn(usize) -> f32) -> Self {
        // Upscaling interpolates with the cubic kernel; downscaling
        // averages with the triangle kernel widened by the scale, which
        // does not ring at the HUD's hard edges.
        let downscale = scale > 1.0;
        let stretch = scale.max(1.0);
        let radius = if downscale { stretch } else { 2.0 };
        let taps = (2.0 * radius).ceil() as usize + 1;
        let last = source_len.saturating_sub(1) as isize;
        let mut starts = Vec::with_capacity(len);
        let mut weights = Vec::with_capacity(len * taps);
        let mut raw = vec![0.0f32; taps];
        let mut index = vec![0usize; taps];
        for i in 0..len {
            let centre = position(i) - 0.5;
            let first = (centre - radius).floor() as isize + 1;
            let mut sum = 0.0;
            for (k, w) in raw.iter_mut().enumerate() {
                let x = first + k as isize;
                let t = (x as f32 - centre) / stretch;
                *w = if downscale {
                    (1.0 - t.abs()).max(0.0)
                } else {
                    cubic(t)
                };
                sum += *w;
                index[k] = x.clamp(0, last) as usize;
            }
            // Taps that fall on the same clamped source pixel are merged,
            // so that every output pixel reads a contiguous window.
            let start = index[0];
            let mut merged = vec![0.0f32; taps];
            for (k, &w) in raw.iter().enumerate() {
                merged[(index[k] - start).min(taps - 1)] += w;
            }
            let norm = if sum.abs() > f32::EPSILON {
                1.0 / sum
            } else {
                1.0
            };
            let mut fixed: Vec<i32> = merged
                .iter()
                .map(|&w| (w * norm * WEIGHT_ONE as f32).round() as i32)
                .collect();
            // Exact unit sum: the rounding error goes to the largest tap.
            let error = WEIGHT_ONE - fixed.iter().sum::<i32>();
            if let Some(top) = fixed.iter_mut().max_by_key(|w| **w) {
                *top += error;
            }
            starts.push(start);
            weights.extend_from_slice(&fixed);
        }
        Self {
            taps,
            starts,
            weights,
        }
    }

    /// The source window `start..start + taps` (clipped to the source by
    /// the caller) and weights of output pixel `i`.
    fn of(&self, i: usize) -> (usize, &[i32]) {
        (
            self.starts[i],
            &self.weights[i * self.taps..(i + 1) * self.taps],
        )
    }
}

/// The two filters of one reference crop.
#[derive(Debug, Clone, PartialEq)]
struct CropFilters {
    roi: RectI,
    anchor: HudAnchor,
    source: (u32, u32),
    x: Filter,
    y: Filter,
    /// Source rows the crop reads.
    rows: (usize, usize),
}

/// An owned RGB crop in reference pixels, filled by
/// [`HudSampler::crop`].
#[derive(Debug, Clone, Default)]
pub struct HudCrop {
    data: Vec<u8>,
    /// Horizontally filtered source rows (fixed point, 3 channels).
    rows: Vec<i32>,
    filters: Option<CropFilters>,
}

/// Cuts reference crops out of the frames of one video ([module
/// docs](self)).
#[derive(Debug, Clone)]
pub struct HudSampler {
    geometry: HudGeometry,
}

impl HudSampler {
    /// A sampler for a video with `geometry`.
    pub const fn new(geometry: HudGeometry) -> Self {
        Self { geometry }
    }

    /// The video's HUD geometry.
    pub const fn geometry(&self) -> &HudGeometry {
        &self.geometry
    }

    /// The reference rectangle `roi` of `frame` (the whole source frame)
    /// for elements anchored at `anchor`, as a view of `roi`'s size.
    ///
    /// The identity mapping (a 1080p recording) returns the source pixels
    /// themselves without a copy; every other layout resamples into `buf`.
    /// Parts of `roi` outside the frame repeat its edge pixels.
    pub fn crop<'f: 'b, 'b>(
        &self,
        frame: &RgbView<'f>,
        roi: RectI,
        anchor: HudAnchor,
        buf: &'b mut HudCrop,
    ) -> RgbView<'b> {
        if self.geometry.is_reference()
            && let Some(view) = frame.sub(roi)
        {
            return view;
        }
        let source = (frame.width(), frame.height());
        let HudCrop {
            data,
            rows,
            filters,
        } = buf;
        if filters
            .as_ref()
            .is_some_and(|f| f.roi != roi || f.anchor != anchor || f.source != source)
        {
            *filters = None;
        }
        let f = filters.get_or_insert_with(|| self.filters(roi, anchor, source));
        resample(frame, f, rows, data);
        RgbView::packed(data, roi.width(), roi.height())
    }

    /// The filters of reference rectangle `roi` for `anchor` on a source of
    /// `source` pixels.
    fn filters(&self, roi: RectI, anchor: HudAnchor, source: (u32, u32)) -> CropFilters {
        let g = &self.geometry;
        let x = Filter::new(roi.width() as usize, g.sx, source.0 as usize, |i| {
            g.source_x(roi.x0 as f32 + i as f32 + 0.5, anchor)
        });
        let y = Filter::new(roi.height() as usize, g.sy, source.1 as usize, |i| {
            g.source_y(roi.y0 as f32 + i as f32 + 0.5)
        });
        let first = y.starts.iter().copied().min().unwrap_or(0);
        let last = y
            .starts
            .iter()
            .map(|&s| s + y.taps)
            .max()
            .unwrap_or(0)
            .min(source.1 as usize);
        CropFilters {
            roi,
            anchor,
            source,
            x,
            y,
            rows: (first, last.max(first)),
        }
    }

    /// A rectangle in reference pixels (a detection box) in source pixels,
    /// for elements anchored at `anchor`, rounded outward. Unless the
    /// mapping is the identity, it grows by one source pixel on every
    /// side: a stroke the detector found at the edge of a resampled crop
    /// pixel came from source pixels up to the filter's reach around it
    /// (measured on synthetic 720p chat lines: the anti-aliased bottom row
    /// of a name otherwise stayed out of its box).
    pub fn to_source(&self, rect: RectI, anchor: HudAnchor) -> RectI {
        let r = self.geometry.to_source(rect, anchor);
        if self.geometry.is_reference() {
            return r;
        }
        RectI {
            x0: r.x0 - SOURCE_MARGIN,
            y0: r.y0 - SOURCE_MARGIN,
            x1: r.x1 + SOURCE_MARGIN,
            y1: r.y1 + SOURCE_MARGIN,
        }
    }
}

/// Resamples `frame` through `f` into `out` (packed RGB of the crop's
/// size), with `rows` as the horizontally filtered intermediate.
fn resample(frame: &RgbView<'_>, f: &CropFilters, rows: &mut Vec<i32>, out: &mut Vec<u8>) {
    let width = f.x.starts.len();
    let height = f.y.starts.len();
    let (r0, r1) = f.rows;
    let source_w = frame.width() as usize;
    rows.clear();
    rows.resize((r1 - r0) * width * 3, 0);
    for (sy, line) in (r0..r1).zip(rows.chunks_exact_mut(width * 3)) {
        let src = frame.row(sy as u32);
        for (i, px) in line.as_chunks_mut::<3>().0.iter_mut().enumerate() {
            let (start, weights) = f.x.of(i);
            let mut acc = [0i32; 3];
            for (k, &w) in weights.iter().enumerate() {
                let x = (start + k).min(source_w - 1);
                let p = &src[3 * x..3 * x + 3];
                acc[0] += w * i32::from(p[0]);
                acc[1] += w * i32::from(p[1]);
                acc[2] += w * i32::from(p[2]);
            }
            px.copy_from_slice(&acc);
        }
    }
    out.clear();
    out.resize(width * height * 3, 0);
    let round = 1i64 << (2 * WEIGHT_BITS - 1);
    for (j, line) in out.chunks_exact_mut(width * 3).enumerate() {
        let (start, weights) = f.y.of(j);
        for (c, value) in line.iter_mut().enumerate() {
            let mut acc = 0i64;
            for (k, &w) in weights.iter().enumerate() {
                let row = (start + k).clamp(r0, r1 - 1) - r0;
                acc += i64::from(w) * i64::from(rows[row * width * 3 + c]);
            }
            *value = ((acc + round) >> (2 * WEIGHT_BITS)).clamp(0, 255) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use blurrd_core::{HudAnchor, HudGeometry, HudLayout, RectI};

    use super::{HudCrop, HudSampler, cubic};
    use crate::image::RgbView;

    /// A smooth test image: every channel a different slow wave of the
    /// reference coordinates mapped through `g` for `anchor`.
    fn image(g: &HudGeometry, w: u32, h: u32, anchor: HudAnchor) -> Vec<u8> {
        let mut data = vec![0u8; 3 * w as usize * h as usize];
        for y in 0..h {
            for x in 0..w {
                let rx = g.reference_x(x as f32 + 0.5, anchor);
                let ry = g.reference_y(y as f32 + 0.5);
                let i = 3 * (y * w + x) as usize;
                data[i] = (128.0 + 100.0 * (rx / 37.0).sin()) as u8;
                data[i + 1] = (128.0 + 100.0 * (ry / 23.0).cos()) as u8;
                data[i + 2] = (128.0 + 100.0 * ((rx + ry) / 51.0).sin()) as u8;
            }
        }
        data
    }

    #[test]
    fn the_kernel_interpolates() {
        assert!((cubic(0.0) - 1.0).abs() < 1e-6);
        assert!(cubic(1.0).abs() < 1e-6 && cubic(2.0).abs() < 1e-6);
        assert!(cubic(0.5) > 0.5 && cubic(1.5) < 0.0);
    }

    #[test]
    fn the_reference_layout_is_read_without_a_copy() {
        let g = HudGeometry::for_layout(1920, 1080, HudLayout::Auto);
        let data = image(&g, 1920, 1080, HudAnchor::Right);
        let frame = RgbView::new(&data, 1920, 1080, 3 * 1920).unwrap();
        let roi = RectI::new(660, 80, 1920, 412).unwrap();
        let mut buf = HudCrop::default();
        let crop = HudSampler::new(g).crop(&frame, roi, HudAnchor::Right, &mut buf);
        assert_eq!((crop.width(), crop.height()), (1260, 332));
        assert_eq!(crop.pixel(0, 0), frame.pixel(660, 80));
        assert_eq!(crop.pixel(1259, 331), frame.pixel(1919, 411));
        assert_eq!(buf.data.len(), 0);
    }

    #[test]
    fn every_layout_samples_the_same_reference_crop() {
        // The feed crop, a centred banner area and the left panel.
        let crops = [
            (RectI::new(660, 80, 1920, 412).unwrap(), HudAnchor::Right),
            (RectI::new(660, 250, 1260, 400).unwrap(), HudAnchor::Center),
            (RectI::new(0, 550, 780, 1040).unwrap(), HudAnchor::Left),
        ];
        let reference = HudGeometry::for_layout(1920, 1080, HudLayout::Auto);
        let expected = image(&reference, 1920, 1080, HudAnchor::Left);
        let layouts = [
            (1280, 720, HudLayout::Auto),
            (2560, 1440, HudLayout::Auto),
            (3840, 2160, HudLayout::Auto),
            (1280, 960, HudLayout::Auto),
            (1920, 1200, HudLayout::Auto),
            (1920, 1080, HudLayout::Pillarbox { bar_px: 240 }),
            (1920, 1080, HudLayout::Stretched4x3),
        ];
        for (w, h, layout) in layouts {
            let g = HudGeometry::for_layout(w, h, layout);
            for (roi, anchor) in crops {
                let data = image(&g, w, h, anchor);
                let frame = RgbView::new(&data, w, h, 3 * w as usize).unwrap();
                let mut buf = HudCrop::default();
                let crop = HudSampler::new(g.clone()).crop(&frame, roi, anchor, &mut buf);
                // Away from the crop border, the smooth image comes back
                // within a few code values.
                let mut worst = 0u8;
                for y in 4..roi.height() - 4 {
                    for x in 4..roi.width() - 4 {
                        // In the reference layout every anchor is the
                        // identity, so one reference image serves both.
                        let (rx, ry) = (roi.x0 as u32 + x, roi.y0 as u32 + y);
                        let i = 3 * (ry * 1920 + rx) as usize;
                        let got = crop.pixel(x, y);
                        for (want, got) in expected[i..i + 3].iter().zip(got) {
                            worst = worst.max(want.abs_diff(got));
                        }
                    }
                }
                assert!(worst <= 4, "{w}x{h} {layout:?} {anchor:?}: {worst}");
            }
        }
    }
}
