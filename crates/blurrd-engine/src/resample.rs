//! Resampling RGB crops onto the detectors' reference grid.
//!
//! The detectors work in reference pixels of a 1920 × 1080 frame. A crop
//! of a recording at another size is resampled with a separable bicubic
//! filter (Keys kernel, `a = -0.6`, the coefficient of FFmpeg's default
//! bicubic scaler); when the crop is reduced, the kernel is widened by the
//! reduction factor so every source pixel contributes (area-correct
//! downscaling, as swscale does).

/// Bicubic kernel coefficient (FFmpeg's `SWS_BICUBIC` default).
const A: f32 = -0.6;

/// Keys' cubic convolution kernel.
fn cubic(x: f32) -> f32 {
    let x = x.abs();
    if x < 1.0 {
        ((A + 2.0) * x - (A + 3.0)) * x * x + 1.0
    } else if x < 2.0 {
        ((A * x - 5.0 * A) * x + 8.0 * A) * x - 4.0 * A
    } else {
        0.0
    }
}

/// The filter taps of one output sample: the first source index and one
/// weight per source sample from there.
#[derive(Debug, Clone, Default)]
struct Taps {
    start: usize,
    weights: Vec<f32>,
}

/// Taps for `out` output samples whose centres lie at source coordinate
/// `centre(i)` (in source pixel units, pixel `j` covering `j..j + 1`),
/// `scale` output samples per source sample, over `source` samples (edges
/// replicated).
fn taps(out: usize, source: usize, scale: f32, centre: impl Fn(usize) -> f32) -> Vec<Taps> {
    let stretch = (1.0 / scale).max(1.0);
    let support = 2.0 * stretch;
    (0..out)
        .map(|i| {
            let c = centre(i) - 0.5;
            let first = (c - support).floor() as i64 + 1;
            let last = (c + support).ceil() as i64 - 1;
            let mut weights: Vec<(usize, f32)> = (first..=last)
                .map(|j| {
                    let w = cubic((j as f32 - c) / stretch);
                    (j.clamp(0, source as i64 - 1) as usize, w)
                })
                .collect();
            let total: f32 = weights.iter().map(|w| w.1).sum();
            if total.abs() > f32::EPSILON {
                for w in &mut weights {
                    w.1 /= total;
                }
            }
            // Fold replicated edge samples into a contiguous run.
            let start = weights.iter().map(|w| w.0).min().unwrap_or(0);
            let end = weights.iter().map(|w| w.0).max().unwrap_or(0);
            let mut run = vec![0.0; end - start + 1];
            for (j, w) in weights {
                run[j - start] += w;
            }
            Taps {
                start,
                weights: run,
            }
        })
        .collect()
}

/// A source rectangle of packed RGB pixels.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RgbSource<'a> {
    /// The pixels, row after row, `3 × width` bytes per row.
    pub(crate) data: &'a [u8],
    /// Width in pixels.
    pub(crate) width: usize,
    /// Height in pixels.
    pub(crate) height: usize,
}

/// Resamples `source` into `out_w × out_h` packed RGB pixels in `out`.
/// Output column `i` is centred at source x `x_of(i)`, row `j` at source y
/// `y_of(j)` (source pixel units); `scale_x` and `scale_y` are output
/// samples per source sample.
#[allow(
    clippy::too_many_arguments,
    reason = "a plain function over the two separable axes reads better than a parameter struct"
)]
pub(crate) fn resample(
    source: RgbSource<'_>,
    out_w: usize,
    out_h: usize,
    scale_x: f32,
    scale_y: f32,
    x_of: impl Fn(usize) -> f32,
    y_of: impl Fn(usize) -> f32,
    out: &mut Vec<u8>,
) {
    out.clear();
    out.resize(3 * out_w * out_h, 0);
    if source.width == 0 || source.height == 0 {
        return;
    }
    let columns = taps(out_w, source.width, scale_x, x_of);
    let rows = taps(out_h, source.height, scale_y, y_of);
    // Horizontal pass for every source row a vertical tap needs.
    let first_row = rows.iter().map(|t| t.start).min().unwrap_or(0);
    let last_row = rows
        .iter()
        .map(|t| t.start + t.weights.len())
        .max()
        .unwrap_or(0);
    let mut horizontal = vec![0.0f32; 3 * out_w * (last_row - first_row)];
    for (r, line) in horizontal.chunks_exact_mut(3 * out_w).enumerate() {
        let row = &source.data[3 * source.width * (first_row + r)..][..3 * source.width];
        for (px, tap) in line.as_chunks_mut::<3>().0.iter_mut().zip(&columns) {
            let mut acc = [0.0f32; 3];
            for (k, &w) in tap.weights.iter().enumerate() {
                let s = &row[3 * (tap.start + k)..][..3];
                for c in 0..3 {
                    acc[c] += w * f32::from(s[c]);
                }
            }
            px.copy_from_slice(&acc);
        }
    }
    for (line, tap) in out.chunks_exact_mut(3 * out_w).zip(&rows) {
        for (x, px) in line.iter_mut().enumerate() {
            let mut acc = 0.0f32;
            for (k, &w) in tap.weights.iter().enumerate() {
                acc += w * horizontal[3 * out_w * (tap.start + k - first_row) + x];
            }
            *px = acc.round().clamp(0.0, 255.0) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{RgbSource, cubic, resample};

    #[test]
    fn the_kernel_interpolates() {
        assert!((cubic(0.0) - 1.0).abs() < 1e-6);
        assert!(cubic(1.0).abs() < 1e-6 && cubic(2.0).abs() < 1e-6);
    }

    #[test]
    fn identity_and_flat_areas_survive() {
        let data: Vec<u8> = (0..4 * 3 * 3).map(|i| (i * 7 % 256) as u8).collect();
        let source = RgbSource {
            data: &data,
            width: 4,
            height: 3,
        };
        let mut out = Vec::new();
        let centre = |i: usize| i as f32 + 0.5;
        resample(source, 4, 3, 1.0, 1.0, centre, centre, &mut out);
        assert_eq!(out, data, "1:1 at pixel centres is the identity");

        let flat = vec![200u8; 8 * 8 * 3];
        let source = RgbSource {
            data: &flat,
            width: 8,
            height: 8,
        };
        for (w, scale) in [(16, 2.0), (3, 0.375)] {
            let at = move |i: usize| (i as f32 + 0.5) / scale;
            resample(source, w, w, scale, scale, at, at, &mut out);
            assert!(out.iter().all(|&v| v == 200), "{w}");
        }
    }

    #[test]
    fn downscaling_averages_fine_detail() {
        // Alternating black and white columns average to grey at half size.
        let data: Vec<u8> = (0..16 * 4)
            .flat_map(|i| {
                let v = if i % 2 == 0 { 0 } else { 255 };
                [v, v, v]
            })
            .collect();
        let source = RgbSource {
            data: &data,
            width: 16,
            height: 4,
        };
        let mut out = Vec::new();
        let at = |i: usize| (i as f32 + 0.5) * 2.0;
        resample(source, 8, 2, 0.5, 0.5, at, at, &mut out);
        for &v in &out[3..out.len() - 3] {
            assert!((100..=155).contains(&v), "{v}");
        }
    }
}
