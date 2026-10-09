//! Resampling RGB crops into the models' normalised planar input.
//!
//! Both models take planar float input in blue, green, red order. Crops
//! are resized with a separable filter with half-pixel centres: bicubic
//! when enlarging (the cubic convolution kernel with `a = -0.75`, as
//! OpenCV's `INTER_CUBIC`), a triangle filter as wide as the scale when
//! shrinking, so thin strokes of high-resolution sources are averaged
//! instead of skipped. At scale 1 the filter is the identity and the
//! pixels pass unchanged.
//!
//! PaddleOCR itself enlarges bilinearly. The glyphs of 720p and 1080p
//! HUDs are enlarged 2 to 5 times on their way into the models, and with
//! bicubic weights their edges stay sharper: on the synthetic HUD matrix
//! of milestone M16 it raised the kill-feed names read at 720p from 88 to
//! 94 % (all layouts 95.9 to 96.5 %).

use crate::image::RgbView;

/// The cubic convolution kernel with `a = -0.75` at distance `d` (in
/// source samples) from the centre; 0 beyond 2 samples.
fn cubic(d: f32) -> f32 {
    const A: f32 = -0.75;
    let d = d.abs();
    if d <= 1.0 {
        ((A + 2.0) * d - (A + 3.0)) * d * d + 1.0
    } else if d < 2.0 {
        ((A * d - 5.0 * A) * d + 8.0 * A) * d - 4.0 * A
    } else {
        0.0
    }
}

/// Filter taps of one axis: for every output position, `width` source
/// indices starting at `first` with their weights.
#[derive(Debug, Clone, PartialEq)]
struct Taps {
    width: usize,
    first: Vec<usize>,
    weights: Vec<f32>,
}

impl Taps {
    /// Taps that map `src` samples onto `dst` samples.
    fn new(src: u32, dst: u32) -> Self {
        let scale = dst as f32 / src as f32;
        // Support of the filter in source samples (radius): four taps when
        // enlarging (bicubic), up to 2 / scale + 1 when shrinking. The
        // stored window is never wider than there are samples.
        let enlarging = scale >= 1.0;
        let radius = if enlarging { 2.0 } else { 1.0 / scale };
        let support = if enlarging {
            4
        } else {
            (2.0 * radius).ceil() as usize + 1
        };
        let width = support.min(src as usize);
        let mut first = Vec::with_capacity(dst as usize);
        let mut weights = vec![0.0f32; width * dst as usize];
        let mut kernel = vec![0.0f32; support];
        let last = i64::from(src) - 1;
        for i in 0..dst as usize {
            let centre = (i as f32 + 0.5) / scale - 0.5;
            let start = (centre - radius).floor() as i64 + 1;
            let mut sum = 0.0f32;
            for (k, w) in kernel.iter_mut().enumerate() {
                let x = (start + k as i64) as f32;
                *w = if enlarging {
                    cubic(x - centre)
                } else {
                    (1.0 - (x - centre).abs() / radius).max(0.0)
                };
                sum += *w;
            }
            // The window is moved inside the image; taps beyond the edges
            // fold back onto the edge sample, like OpenCV's replicated
            // border.
            let clamped = start.clamp(0, (last - width as i64 + 1).max(0));
            let taps = &mut weights[i * width..(i + 1) * width];
            for (k, &w) in kernel.iter().enumerate() {
                let x = (start + k as i64).clamp(0, last);
                let slot = (x - clamped).clamp(0, width as i64 - 1) as usize;
                taps[slot] += w;
            }
            if sum > 0.0 {
                for w in taps.iter_mut() {
                    *w /= sum;
                }
            }
            first.push(clamped as usize);
        }
        Self {
            width,
            first,
            weights,
        }
    }

    /// The source indices and weights of output position `i`.
    fn get(&self, i: usize) -> (usize, &[f32]) {
        (
            self.first[i],
            &self.weights[i * self.width..(i + 1) * self.width],
        )
    }
}

/// Per-channel affine normalisation `value × scale + offset`, applied to
/// 8-bit values, in the model's channel order (blue, green, red).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Normalise {
    pub(crate) scale: [f32; 3],
    pub(crate) offset: [f32; 3],
}

/// Resizes `image` to `out_w` × `out_h` and writes it normalised into the
/// top-left of three planes of `stride` × rows floats (`planes` holds the
/// blue plane, then green, then red, each `plane_len` floats long). Other
/// values of the planes are left untouched.
pub(crate) fn resize_into_planes(
    image: RgbView<'_>,
    out_w: u32,
    out_h: u32,
    planes: &mut [f32],
    stride: usize,
    plane_len: usize,
    norm: Normalise,
) {
    let (src_w, src_h) = (image.width(), image.height());
    let horizontal = Taps::new(src_w, out_w);
    let vertical = Taps::new(src_h, out_h);
    let row_len = 3 * out_w as usize;
    // Horizontal pass over every source row: RGB floats, out_w wide.
    let mut rows = vec![0.0f32; row_len * src_h as usize];
    for (y, out) in rows.chunks_exact_mut(row_len).enumerate() {
        let src = image.row(y as u32);
        for (x, px) in out.as_chunks_mut::<3>().0.iter_mut().enumerate() {
            let (first, weights) = horizontal.get(x);
            let mut acc = [0.0f32; 3];
            for (k, &w) in weights.iter().enumerate() {
                let s = &src[3 * (first + k)..3 * (first + k) + 3];
                acc[0] += w * f32::from(s[0]);
                acc[1] += w * f32::from(s[1]);
                acc[2] += w * f32::from(s[2]);
            }
            *px = acc;
        }
    }
    // Vertical pass, written straight into the three planes.
    let mut blended = vec![0.0f32; row_len];
    for y in 0..out_h as usize {
        let (first, weights) = vertical.get(y);
        blended.fill(0.0);
        for (k, &w) in weights.iter().enumerate() {
            if w == 0.0 {
                continue;
            }
            let src = &rows[(first + k) * row_len..][..row_len];
            for (b, &s) in blended.iter_mut().zip(src) {
                *b += w * s;
            }
        }
        let base = y * stride;
        let (blue, rest) = planes.split_at_mut(plane_len);
        let (green, red) = rest.split_at_mut(plane_len);
        let dst = [
            &mut blue[base..base + out_w as usize],
            &mut green[base..base + out_w as usize],
            &mut red[base..base + out_w as usize],
        ];
        // Input channel order is R, G, B; the planes are B, G, R.
        for (x, px) in blended.as_chunks::<3>().0.iter().enumerate() {
            dst[0][x] = px[2] * norm.scale[0] + norm.offset[0];
            dst[1][x] = px[1] * norm.scale[1] + norm.offset[1];
            dst[2][x] = px[0] * norm.scale[2] + norm.offset[2];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Normalise, Taps, resize_into_planes};
    use crate::image::RgbView;

    const IDENTITY: Normalise = Normalise {
        scale: [1.0; 3],
        offset: [0.0; 3],
    };

    #[test]
    fn scale_one_is_the_identity() {
        let taps = Taps::new(5, 5);
        for i in 0..5 {
            let (first, weights) = taps.get(i);
            let value: f32 = weights
                .iter()
                .enumerate()
                .map(|(k, w)| w * (first + k) as f32)
                .sum();
            assert!((value - i as f32).abs() < 1e-5, "{i}: {value}");
        }
    }

    #[test]
    fn weights_sum_to_one_at_every_scale() {
        for (src, dst) in [(10, 25), (25, 10), (1260, 840), (7, 7), (3, 48), (100, 1)] {
            let taps = Taps::new(src, dst);
            for i in 0..dst as usize {
                let (first, weights) = taps.get(i);
                let sum: f32 = weights.iter().sum();
                assert!((sum - 1.0).abs() < 1e-4, "{src}->{dst} at {i}: {sum}");
                assert!(first + weights.len() <= src as usize);
            }
        }
    }

    #[test]
    fn shrinking_averages_thin_lines() {
        // A one-pixel white line every 4 pixels, shrunk 4x: a bilinear
        // sampler would hit or miss the lines; the triangle filter keeps
        // their mean (64) everywhere.
        let w = 64u32;
        let data: Vec<u8> = (0..w)
            .flat_map(|x| if x % 4 == 0 { [255u8; 3] } else { [0u8; 3] })
            .collect();
        let image = RgbView::new(w, 1, &data).unwrap();
        let mut planes = vec![0.0f32; 3 * 16];
        resize_into_planes(image, 16, 1, &mut planes, 16, 16, IDENTITY);
        for &v in &planes[2..14] {
            assert!((v - 63.75).abs() < 8.0, "{v}");
        }
    }

    #[test]
    fn enlarging_keeps_edges_sharp() {
        // A dark-to-bright step enlarged 4x: bilinear weights would ramp
        // from 32 to 223 over the three outputs around the edge; the cubic
        // kernel is steeper and overshoots a little on both sides.
        let data: Vec<u8> = (0..8u8)
            .flat_map(|x| if x < 4 { [0u8; 3] } else { [255u8; 3] })
            .collect();
        let image = RgbView::new(8, 1, &data).unwrap();
        let mut planes = vec![0.0f32; 3 * 32];
        resize_into_planes(image, 32, 1, &mut planes, 32, 32, IDENTITY);
        let ramp = planes[17] - planes[14];
        assert!(ramp > 191.25, "{ramp}");
        assert!(planes[12] < 0.0 && planes[19] > 255.0);
    }

    #[test]
    fn writes_bgr_planes() {
        let data = [10u8, 20, 30];
        let image = RgbView::new(1, 1, &data).unwrap();
        let mut planes = vec![-1.0f32; 3 * 4];
        resize_into_planes(image, 1, 1, &mut planes, 2, 4, IDENTITY);
        assert_eq!(
            planes,
            [
                30.0, -1.0, -1.0, -1.0, 20.0, -1.0, -1.0, -1.0, 10.0, -1.0, -1.0, -1.0
            ]
        );
    }
}
