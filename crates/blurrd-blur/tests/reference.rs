//! The optimised CPU blur against a straightforward reference.
//!
//! The reference follows the algorithm literally: area-average downsample
//! on the block grid, per-block noise, bilinear upsample of the whole box,
//! then a separable gaussian with a replicated border, and a mix through
//! the box's mask. The optimised backend reorders these linear steps and
//! vectorises them, so results may differ by rounding, but never by more
//! than one code value.

#![allow(
    clippy::many_single_char_names,
    clippy::cast_possible_wrap,
    clippy::unwrap_used,
    reason = "test code: short pixel coordinate names, small synthetic sizes, panics are failures"
)]

mod support;

use blurrd_blur::{
    BlurBackend, CpuBlur, GAUSSIAN_SIGMA_PER_BLOCK, NOISE_AMPLITUDE, PIXEL_BLUR_SIGMA_PER_BLOCK,
    PixelParams, PreparedBox, YuvColour, prepare_boxes,
};
use blurrd_core::{BlurBox, BlurParams, BlurStyle, RectI, Region, Strength};
use proptest::prelude::*;
use support::{Format, OwnedFrame};

/// Cells of the block grid along one axis, cut to `lo..hi`.
struct Cells {
    bounds: Vec<i32>,
    centres: Vec<f32>,
    first: i64,
}

fn cells(origin: f32, block: f32, lo: i32, hi: i32) -> Cells {
    let (origin, block) = (f64::from(origin), f64::from(block));
    let edge = |k: i64| (origin + k as f64 * block + 0.5).floor() as i32;
    let mut k = ((f64::from(lo) - origin) / block).floor() as i64 - 1;
    while edge(k + 1) <= lo {
        k += 1;
    }
    let first = k;
    let mut bounds = vec![lo];
    let mut centres = vec![];
    while edge(k) < hi {
        bounds.push(edge(k + 1).min(hi));
        centres.push((edge(k) + edge(k + 1)) as f32 * 0.5);
        k += 1;
    }
    Cells {
        bounds,
        centres,
        first,
    }
}

/// The interpolation weights of sample position `p` between cell centres.
fn bilinear(c: &[f32], p: f32) -> (usize, usize, f32) {
    let last = c.len() - 1;
    if p <= c[0] {
        return (0, 0, 0.0);
    }
    let j = (0..last).rev().find(|&j| c[j] <= p).unwrap_or(0);
    if c[last] <= p {
        (last, last, 0.0)
    } else {
        (j, j + 1, (p - c[j]) / (c[j + 1] - c[j]))
    }
}

fn noise(seed: u32, kx: i64, ky: i64) -> f32 {
    let mut z = u64::from(seed)
        ^ (kx as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (ky as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    ((z >> 40) as f32 / (1u32 << 23) as f32) - 1.0
}

fn gaussian(sigma: f32) -> Vec<f32> {
    let radius = (3.0 * sigma).ceil().max(1.0) as i32;
    let w: Vec<f32> = (-radius..=radius)
        .map(|t| (-((t * t) as f32) / (2.0 * sigma * sigma)).exp())
        .collect();
    let sum: f32 = w.iter().sum();
    w.into_iter().map(|v| v / sum).collect()
}

/// The blurred values of one box in channel `c` (row-major over its area).
fn reference_patch(
    source: &OwnedFrame,
    c: usize,
    b: &PreparedBox,
    params: &BlurParams,
) -> (RectI, Vec<f32>) {
    let s = if c == 0 { 1 } else { 2 };
    let p = b.padded();
    let area = RectI::new(p.x0 / s, p.y0 / s, (p.x1 + s - 1) / s, (p.y1 + s - 1) / s).unwrap();
    let (w, h) = (area.width() as usize, area.height() as usize);
    let at = |x: i32, y: i32| f32::from(source.get(c, x as u32, y as u32));
    let block = b.block() / s as f32;
    let (gx, gy) = b.grid_origin();
    let cols = cells(gx as f32 / s as f32, block, area.x0, area.x1);
    let rows = cells(gy as f32 / s as f32, block, area.y0, area.y1);
    let amplitude = NOISE_AMPLITUDE * f32::from(source.format.max()) / 255.0;
    let mut means = vec![vec![0.0f32; cols.centres.len()]; rows.centres.len()];
    for (r, row) in means.iter_mut().enumerate() {
        for (k, mean) in row.iter_mut().enumerate() {
            let (mut sum, mut n) = (0.0f64, 0.0f64);
            for y in rows.bounds[r]..rows.bounds[r + 1] {
                for x in cols.bounds[k]..cols.bounds[k + 1] {
                    sum += f64::from(at(x, y));
                    n += 1.0;
                }
            }
            *mean = (sum / n) as f32;
            if params.noise && c == 0 {
                *mean += amplitude * noise(b.seed(), cols.first + k as i64, rows.first + r as i64);
            }
        }
    }
    let sigma = match params.style {
        BlurStyle::Gaussian => GAUSSIAN_SIGMA_PER_BLOCK * b.block(),
        _ => PIXEL_BLUR_SIGMA_PER_BLOCK * b.block(),
    } / s as f32;
    let mut up = vec![0.0f32; w * h];
    for y in 0..h {
        let (r0, r1, ty) = bilinear(&rows.centres, (area.y0 + y as i32) as f32 + 0.5);
        for x in 0..w {
            let (c0, c1, tx) = bilinear(&cols.centres, (area.x0 + x as i32) as f32 + 0.5);
            let top = means[r0][c0] + (means[r0][c1] - means[r0][c0]) * tx;
            let bottom = means[r1][c0] + (means[r1][c1] - means[r1][c0]) * tx;
            up[y * w + x] = top + (bottom - top) * ty;
        }
    }
    let k = gaussian(sigma);
    let r = (k.len() / 2) as isize;
    let clamp = |i: isize, n: usize| i.clamp(0, n as isize - 1) as usize;
    let mut tmp = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            tmp[y * w + x] = k
                .iter()
                .enumerate()
                .map(|(t, g)| g * up[y * w + clamp(x as isize + t as isize - r, w)])
                .sum();
        }
    }
    let mut out = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            out[y * w + x] = k
                .iter()
                .enumerate()
                .map(|(t, g)| g * tmp[clamp(y as isize + t as isize - r, h) * w + x])
                .sum();
        }
    }
    (area, out)
}

/// The reference blur of `frame` (smooth styles).
fn reference_blur(frame: &mut OwnedFrame, boxes: &[PreparedBox], params: &BlurParams) {
    let source = frame.clone();
    let max = f32::from(frame.format.max());
    for c in 0..3 {
        let patches: Vec<_> = boxes
            .iter()
            .map(|b| reference_patch(&source, c, b, params))
            .collect();
        for (b, (area, patch)) in boxes.iter().zip(patches) {
            let w = area.width() as usize;
            for y in area.y0..area.y1 {
                for x in area.x0..area.x1 {
                    let m = if c == 0 {
                        b.coverage(x, y)
                    } else {
                        b.chroma_coverage(x, y)
                    };
                    let src = f32::from(frame.get(c, x as u32, y as u32));
                    let blurred = patch[(y - area.y0) as usize * w + (x - area.x0) as usize];
                    let v = ((blurred - src) * m + src).clamp(0.0, max) + 0.5;
                    frame.set(c, x as u32, y as u32, v as u16);
                }
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// The optimised backend matches the reference within one code value.
    #[test]
    fn matches_the_reference(
        format in prop::sample::select(Format::ALL.to_vec()),
        style in prop_oneof![Just(BlurStyle::PixelBlur), Just(BlurStyle::Gaussian)],
        strength in prop_oneof![Just(Strength::Low), Just(Strength::Medium), Just(Strength::High)],
        noise_on in any::<bool>(),
        sy in prop_oneof![Just(2.0f32 / 3.0), Just(1.0f32), Just(1.5f32)],
        seed in any::<u64>(),
        boxes in prop::collection::vec(
            (0..90i32, 0..50i32, 1..70i32, 1..30i32, -12..12i32, -12..12i32, any::<u32>(),
             prop_oneof![Just(1.0f32), Just(1.5f32)]),
            1..4,
        ),
    ) {
        let (w, h) = (130u32, 86u32);
        let params = BlurParams { style, strength, noise: noise_on, ..BlurParams::default() };
        let pixel = PixelParams::new(&params, sy, YuvColour::default()).unwrap();
        let boxes: Vec<BlurBox> = boxes
            .into_iter()
            .map(|(x, y, bw, bh, gx, gy, seed, block_scale)| BlurBox {
                rect: RectI::new(x, y, x + bw, y + bh).unwrap(),
                region: Region::KillFeed,
                seed,
                grid_origin: (x + gx, y + gy),
                block_scale,
            })
            .collect();
        let mut prepared = vec![];
        prepare_boxes(&boxes, w, h, &pixel, &mut prepared);
        let source = OwnedFrame::noise(format, w, h, seed);
        let mut fast = source.clone();
        CpuBlur.blur(&mut fast.view(), (0, 0), &prepared, &pixel).unwrap();
        let mut slow = source.clone();
        reference_blur(&mut slow, &prepared, &params);
        for c in 0..3 {
            let (cw, ch) = fast.channel_size(c);
            for y in 0..ch {
                for x in 0..cw {
                    let (a, b) = (fast.get(c, x, y), slow.get(c, x, y));
                    prop_assert!(a.abs_diff(b) <= 1, "channel {c} ({x}, {y}): {a} vs {b}");
                }
            }
        }
    }
}
