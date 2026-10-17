//! Text detection: PP-OCR pre-processing and the DB post-processing.
//!
//! The detection model (DBNet) outputs a text probability per pixel. The
//! post-processing follows PaddleOCR's `DBPostProcess` for horizontal text,
//! written for blurrd's fixed input shapes: binarise the map at
//! [`BIN_THRESHOLD`], take 8-connected components, score each by the mean
//! probability inside its bounding box, keep boxes scoring at least
//! [`BOX_THRESHOLD`] and grow them by the unclip offset
//! `area × UNCLIP_RATIO / perimeter`. HUD text is never rotated, so the
//! boxes are axis-aligned and no polygon clipping is needed.

use blurrd_core::RectI;

use crate::image::RgbView;
use crate::resample::{Normalise, Resampler};

/// Per-channel mean of the detection input, in the model's channel order
/// (blue, green, red, as PaddleOCR feeds OpenCV images).
const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
/// Per-channel standard deviation, same order as [`MEAN`].
const STD: [f32; 3] = [0.229, 0.224, 0.225];

/// Probability above which a pixel counts as text (PaddleOCR `thresh`).
pub const BIN_THRESHOLD: f32 = 0.3;
/// Minimum mean probability of a kept box (PaddleOCR `box_thresh`, 0.6;
/// RapidOCR uses 0.5). blurrd keeps 0.5: a missed name costs more than a
/// spurious box, which recognition then reads as nothing.
pub const BOX_THRESHOLD: f32 = 0.5;
/// How far boxes grow beyond the shrunk text kernel the model predicts
/// (PaddleOCR `unclip_ratio` 1.5; RapidOCR 1.6).
pub const UNCLIP_RATIO: f32 = 1.6;
/// Boxes whose shorter side is below this many detection pixels are noise
/// (PaddleOCR `min_size`).
const MIN_SIZE: u32 = 3;
/// Components are visited up to this many per image (PaddleOCR
/// `max_candidates`).
const MAX_CANDIDATES: usize = 1000;

/// How a zone image was placed into the fixed detection input.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Placement {
    /// Input pixels per image pixel, horizontally.
    pub(crate) scale_x: f32,
    /// Input pixels per image pixel, vertically. It differs from
    /// `scale_x` only for images whose pixels are not square
    /// ([`RgbView::pixel_aspect`]).
    pub(crate) scale_y: f32,
    /// Width of the image inside the input, in input pixels.
    pub(crate) width: u32,
    /// Height of the image inside the input, in input pixels.
    pub(crate) height: u32,
}

/// Scales `image` to fit `content_w` × `content_h` with its natural aspect
/// ratio (the pixel aspect undone, [`RgbView::pixel_aspect`]), writes it
/// normalised into the top-left of a `width` × `height` NCHW input `out`
/// with `resampler` and zeroes the rest (0 is the normalised mean colour).
/// A square-pixel crop of the reference size has scale 1 and passes
/// unchanged.
pub(crate) fn preprocess(
    resampler: &mut Resampler,
    image: RgbView<'_>,
    (content_w, content_h): (u32, u32),
    (width, height): (u32, u32),
    out: &mut [f32],
) -> Placement {
    let natural_w = image.natural_width();
    let scale = (content_w as f32 / natural_w).min(content_h as f32 / image.height() as f32);
    let placed_w = ((natural_w * scale).round() as u32).clamp(1, width);
    let placed_h = ((image.height() as f32 * scale).round() as u32).clamp(1, height);
    let norm = Normalise {
        scale: [0, 1, 2].map(|c| 1.0 / (255.0 * STD[c])),
        offset: [0, 1, 2].map(|c| -MEAN[c] / STD[c]),
    };
    let (w, plane) = (width as usize, width as usize * height as usize);
    // The image overwrites its part of the input; only the rest is zeroed,
    // so a reused input costs no full clear.
    for channel in out.chunks_exact_mut(plane) {
        for row in channel.chunks_exact_mut(w).take(placed_h as usize) {
            row[placed_w as usize..].fill(0.0);
        }
        channel[placed_h as usize * w..].fill(0.0);
    }
    resampler.resize_into_planes(image, placed_w, placed_h, out, w, plane, norm);
    Placement {
        scale_x: placed_w as f32 / image.width() as f32,
        scale_y: placed_h as f32 / image.height() as f32,
        width: placed_w,
        height: placed_h,
    }
}

/// A detected line of text.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextBox {
    /// Where the text is, in pixels of the image passed to
    /// [`OcrEngine::detect`](crate::OcrEngine::detect).
    pub rect: RectI,
    /// Mean text probability inside the box, `0.0..=1.0`.
    pub score: f32,
}

/// The text probability map of one detection, kept for the
/// dictionary-free leak check (text inside a name slot that recognition
/// cannot read is still text).
#[derive(Debug, Clone, PartialEq)]
pub struct ProbMap {
    width: u32,
    height: u32,
    /// Detection pixels per image pixel, horizontally.
    scale_x: f32,
    /// Detection pixels per image pixel, vertically.
    scale_y: f32,
    /// Probabilities quantised to `0..=255`, row by row.
    data: Vec<u8>,
}

impl ProbMap {
    /// Width of the map in detection pixels.
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Height of the map in detection pixels.
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// The mean text probability inside `rect` (image pixels), or 0 when it
    /// lies outside the map.
    pub fn mean_in(&self, rect: RectI) -> f32 {
        self.fold_in(rect, 0u64, |sum, p| sum + u64::from(p))
            .map_or(0.0, |(sum, n)| sum as f32 / (n as f32 * 255.0))
    }

    /// The highest text probability inside `rect` (image pixels), or 0.
    pub fn max_in(&self, rect: RectI) -> f32 {
        self.fold_in(rect, 0u8, u8::max)
            .map_or(0.0, |(max, _)| f32::from(max) / 255.0)
    }

    /// Folds `f` over the map pixels covering `rect`; returns the result and
    /// the number of pixels, or `None` if no pixel is covered.
    fn fold_in<T>(&self, rect: RectI, init: T, f: impl Fn(T, u8) -> T) -> Option<(T, u64)> {
        let map = |v: i32, scale: f32| (v as f32 * scale).floor() as i64;
        let map_end = |v: i32, scale: f32| (v as f32 * scale).ceil() as i64;
        let x0 = map(rect.x0, self.scale_x).clamp(0, i64::from(self.width)) as usize;
        let x1 = map_end(rect.x1, self.scale_x).clamp(0, i64::from(self.width)) as usize;
        let y0 = map(rect.y0, self.scale_y).clamp(0, i64::from(self.height)) as usize;
        let y1 = map_end(rect.y1, self.scale_y).clamp(0, i64::from(self.height)) as usize;
        if x0 >= x1 || y0 >= y1 {
            return None;
        }
        let mut acc = init;
        for y in y0..y1 {
            let line = &self.data[y * self.width as usize..][x0..x1];
            acc = line.iter().fold(acc, |a, &p| f(a, p));
        }
        Some((acc, ((x1 - x0) * (y1 - y0)) as u64))
    }
}

/// The result of one detection: text boxes and the probability map.
#[derive(Debug, Clone, PartialEq)]
pub struct TextMap {
    /// Detected text lines, top to bottom.
    pub boxes: Vec<TextBox>,
    /// Text probability over the image.
    pub prob: ProbMap,
}

/// Turns the model output (`input_w` wide) into text boxes in image
/// pixels. Only the area covered by the image ([`Placement`]) is read.
pub(crate) fn postprocess(
    prob: &[f32],
    input_w: u32,
    placement: Placement,
    image: RectI,
) -> TextMap {
    let (w, h) = (placement.width as usize, placement.height as usize);
    let stride = input_w as usize;
    let mut data = Vec::with_capacity(w * h);
    for y in 0..h {
        data.extend(
            prob[y * stride..y * stride + w]
                .iter()
                .map(|&p| (p.clamp(0.0, 1.0) * 255.0).round() as u8),
        );
    }
    let threshold = (BIN_THRESHOLD * 255.0) as u8;
    let mut boxes = Vec::new();
    for (x0, y0, x1, y1) in components(&data, w, h, threshold)
        .into_iter()
        .take(MAX_CANDIDATES)
    {
        let (bw, bh) = (x1 - x0, y1 - y0);
        if bw.min(bh) < MIN_SIZE as usize {
            continue;
        }
        let sum: u64 = (y0..y1)
            .map(|y| {
                data[y * w + x0..y * w + x1]
                    .iter()
                    .map(|&p| u64::from(p))
                    .sum::<u64>()
            })
            .sum();
        let score = sum as f32 / ((bw * bh) as f32 * 255.0);
        if score < BOX_THRESHOLD {
            continue;
        }
        let area = (bw * bh) as f32;
        let perimeter = 2.0 * (bw + bh) as f32;
        let grow = area * UNCLIP_RATIO / perimeter;
        if (bw.min(bh) as f32 + 2.0 * grow) < (MIN_SIZE + 2) as f32 {
            continue;
        }
        // Back to image pixels.
        let (sx, sy) = (placement.scale_x, placement.scale_y);
        let rect = RectI::new(
            ((x0 as f32 - grow) / sx).floor() as i32,
            ((y0 as f32 - grow) / sy).floor() as i32,
            ((x1 as f32 + grow) / sx).ceil() as i32,
            ((y1 as f32 + grow) / sy).ceil() as i32,
        )
        .ok()
        .and_then(|r| r.intersection(&image));
        if let Some(rect) = rect {
            boxes.push(TextBox { rect, score });
        }
    }
    boxes.sort_by_key(|b| (b.rect.y0, b.rect.x0));
    TextMap {
        boxes,
        prob: ProbMap {
            width: placement.width,
            height: placement.height,
            scale_x: placement.scale_x,
            scale_y: placement.scale_y,
            data,
        },
    }
}

/// A horizontal run of pixels above the threshold.
#[derive(Debug, Clone, Copy)]
struct Run {
    y: usize,
    x0: usize,
    x1: usize,
}

/// Bounding boxes `(x0, y0, x1, y1)` (half-open) of the 8-connected
/// components of the pixels above `threshold`, in the order their first
/// pixel appears in a row-by-row scan. Runs of each row are joined to the
/// runs of the row above that touch them (union-find), so the cost grows
/// with the number of runs, not of pixels.
fn components(data: &[u8], w: usize, h: usize, threshold: u8) -> Vec<(usize, usize, usize, usize)> {
    let mut runs: Vec<Run> = Vec::new();
    let mut parent: Vec<usize> = Vec::new();
    let mut previous = 0..0;
    for y in 0..h {
        let line = &data[y * w..(y + 1) * w];
        let row_start = runs.len();
        let mut x = 0;
        while x < w {
            if line[x] <= threshold {
                x += 1;
                continue;
            }
            let x0 = x;
            while x < w && line[x] > threshold {
                x += 1;
            }
            let id = runs.len();
            runs.push(Run { y, x0, x1: x });
            parent.push(id);
            // 8-connectivity: runs of the row above that overlap
            // [x0 - 1, x + 1).
            for above in previous.clone() {
                let r: Run = runs[above];
                if r.x0 <= x && r.x1 >= x0 {
                    union(&mut parent, above, id);
                }
            }
        }
        previous = row_start..runs.len();
    }
    let mut order: Vec<usize> = Vec::new();
    let mut bounds: Vec<Option<(usize, usize, usize, usize)>> = vec![None; runs.len()];
    for (id, run) in runs.iter().enumerate() {
        let root = find(&mut parent, id);
        let b = bounds[root].get_or_insert_with(|| {
            order.push(root);
            (run.x0, run.y, run.x1, run.y + 1)
        });
        b.0 = b.0.min(run.x0);
        b.1 = b.1.min(run.y);
        b.2 = b.2.max(run.x1);
        b.3 = b.3.max(run.y + 1);
    }
    order.into_iter().filter_map(|root| bounds[root]).collect()
}

/// The root of `i` (with path halving).
fn find(parent: &mut [usize], mut i: usize) -> usize {
    while parent[i] != i {
        parent[i] = parent[parent[i]];
        i = parent[i];
    }
    i
}

/// Joins the sets of `a` and `b` (the smaller root wins, so roots are the
/// earliest runs).
fn union(parent: &mut [usize], a: usize, b: usize) {
    let (ra, rb) = (find(parent, a), find(parent, b));
    if ra != rb {
        let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
        parent[hi] = lo;
    }
}

#[cfg(test)]
mod tests {
    use blurrd_core::RectI;

    use super::{Placement, components, postprocess, preprocess};
    use crate::image::RgbView;
    use crate::resample::Resampler;

    #[test]
    fn preprocess_keeps_aspect_and_normalises() {
        // 4x2 white image into an 8x8 input with 8x8 content: placed 8x4,
        // the rest is padding.
        let data = vec![255u8; 4 * 2 * 3];
        let image = RgbView::new(4, 2, &data).unwrap();
        let mut out = vec![9.0f32; 3 * 8 * 8];
        // One resampler and one input for every call, as the engine keeps
        // them: what an earlier call wrote never shows through.
        let mut resampler = Resampler::default();
        let placement = preprocess(&mut resampler, image, (8, 8), (8, 8), &mut out);
        assert_eq!(
            placement,
            Placement {
                scale_x: 2.0,
                scale_y: 2.0,
                width: 8,
                height: 4
            }
        );
        // Blue plane, white pixel: (1 - 0.485) / 0.229.
        assert!((out[0] - (1.0 - 0.485) / 0.229).abs() < 1e-4);
        // Red plane (third), white pixel.
        assert!((out[2 * 64] - (1.0 - 0.406) / 0.225).abs() < 1e-4);
        // Padding below the image.
        assert_eq!(out[5 * 8], 0.0);
        // Content smaller than the input: scale 1, no resampling.
        let placement = preprocess(&mut resampler, image, (4, 2), (8, 8), &mut out);
        assert_eq!(
            (placement.scale_x, placement.width, placement.height),
            (1.0, 4, 2)
        );
        assert_eq!(out[4], 0.0);
        // Stretched pixels (aspect 0.5): the 4x2 image is 2x2 in natural
        // proportions and fills 8x8 at scale 4 vertically, 2 horizontally.
        let placement = preprocess(
            &mut resampler,
            image.with_pixel_aspect(0.5),
            (8, 8),
            (8, 8),
            &mut out,
        );
        assert_eq!(
            placement,
            Placement {
                scale_x: 2.0,
                scale_y: 4.0,
                width: 8,
                height: 8
            }
        );
    }

    #[test]
    fn joins_runs_into_eight_connected_components() {
        // Two diagonal pixels touch; a separate blob further right.
        #[rustfmt::skip]
        let data = [
            9, 0, 0, 0, 9,
            0, 9, 0, 0, 9,
            0, 0, 0, 0, 0,
        ];
        assert_eq!(components(&data, 5, 3, 0), [(0, 0, 2, 2), (4, 0, 5, 2)]);
        // A U shape whose arms meet only at the bottom is one component.
        #[rustfmt::skip]
        let u = [
            9, 0, 9,
            9, 0, 9,
            9, 9, 9,
        ];
        assert_eq!(components(&u, 3, 3, 0), [(0, 0, 3, 3)]);
    }

    #[test]
    fn finds_boxes_and_maps_them_back() {
        // 40x20 input, image placed at scale 2 over 40x20.
        let (w, h) = (40usize, 20usize);
        let mut prob = vec![0.0f32; w * h];
        for y in 8..12 {
            for x in 4..30 {
                prob[y * w + x] = 0.9;
            }
        }
        // A faint blob that scores below the box threshold.
        for y in 2..5 {
            for x in 34..38 {
                prob[y * w + x] = 0.35;
            }
        }
        let placement = Placement {
            scale_x: 2.0,
            scale_y: 2.0,
            width: 40,
            height: 20,
        };
        let image = RectI::new(0, 0, 20, 10).unwrap();
        let map = postprocess(&prob, 40, placement, image);
        assert_eq!(map.boxes.len(), 1);
        let text = map.boxes[0];
        assert!((text.score - 0.9).abs() < 3e-3);
        // Kernel 26x4 at x 4..30, y 8..12 grows by 26*4*1.6/60 = 2.77 px,
        // then halves into image pixels.
        assert_eq!(text.rect, RectI::new(0, 2, 17, 8).unwrap());
        assert!(map.prob.mean_in(RectI::new(2, 4, 15, 6).unwrap()) > 0.85);
        assert!(map.prob.max_in(RectI::new(17, 1, 19, 2).unwrap()) > 0.3);
        assert_eq!(map.prob.mean_in(RectI::new(30, 30, 40, 40).unwrap()), 0.0);
        // The same map from an image whose pixels were enlarged 4x
        // horizontally and 2x vertically: boxes map back per axis.
        let anamorphic = Placement {
            scale_x: 4.0,
            ..placement
        };
        let map = postprocess(&prob, 40, anamorphic, RectI::new(0, 0, 10, 10).unwrap());
        assert_eq!(map.boxes[0].rect, RectI::new(0, 2, 9, 8).unwrap());
        assert!(map.prob.mean_in(RectI::new(1, 4, 7, 6).unwrap()) > 0.85);
    }
}
