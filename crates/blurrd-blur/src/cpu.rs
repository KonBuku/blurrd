//! The CPU blur backend.

#![allow(
    clippy::cast_possible_wrap,
    reason = "box and plane sizes come from i32 frame coordinates, so index arithmetic cannot wrap"
)]

use blurrd_core::RectI;

use crate::frame::{Channel, Subsampling};
use crate::params::{Fill, NOISE_AMPLITUDE};
use crate::{BlurBackend, BlurError, PixelParams, PreparedBox, YuvFrameMut};

/// Blurs boxes on the CPU, directly on the YUV planes.
///
/// Per box and channel: area-average downsample on the block grid anchored
/// at the box's grid origin (cells cut by the padded box average only their
/// pixels inside it), optional per-block noise, then the fill (bilinear
/// upsample and separable gaussian with a replicated border inside the
/// padded box, hard blocks, or a solid colour), and finally a mix with the
/// source through the box's mask. Chroma uses the same steps at half
/// resolution with the max-pooled mask.
///
/// Every box is computed from the source pixels of its padded box only; the
/// results are composited afterwards in box order, so overlapping boxes do
/// not read each other's output.
#[derive(Debug, Clone, Copy, Default)]
pub struct CpuBlur;

impl BlurBackend for CpuBlur {
    fn blur(
        &self,
        frame: &mut YuvFrameMut<'_>,
        origin: (i32, i32),
        boxes: &[PreparedBox],
        params: &PixelParams,
    ) -> Result<(), BlurError> {
        frame.validate()?;
        let view = view_rect(frame, origin)?;
        if let Some(b) = boxes
            .iter()
            .find(|b| b.padded().intersection(&view) != Some(b.padded()))
        {
            return Err(BlurError::BoxOutsideView {
                padded: b.padded(),
                view,
            });
        }
        if boxes.is_empty() {
            return Ok(());
        }
        let mut scratch = Scratch::default();
        for (index, channel) in frame.channels().into_iter().enumerate() {
            let s = channel.subsampling.factor();
            let offset = (origin.0 / s, origin.1 / s);
            scratch.patches.clear();
            let source: &[u8] = frame.plane_data(channel.plane);
            for b in boxes {
                let area = Area::of(b, channel.subsampling);
                let job = PatchJob {
                    channel: &channel,
                    data: source,
                    offset,
                    area,
                    index,
                    params,
                    b,
                };
                job.fill(&mut scratch);
            }
            let target = frame.plane_data(channel.plane);
            let mut start = 0;
            for b in boxes {
                let area = Area::of(b, channel.subsampling);
                let len = area.len();
                composite(
                    &channel,
                    target,
                    offset,
                    area,
                    b,
                    &scratch.patches[start..start + len],
                );
                start += len;
            }
        }
        Ok(())
    }
}

/// The part of the full frame that `frame` shows, given its origin.
fn view_rect(frame: &YuvFrameMut<'_>, origin: (i32, i32)) -> Result<RectI, BlurError> {
    if origin.0 % 2 != 0 || origin.1 % 2 != 0 {
        return Err(BlurError::OddOrigin {
            x: origin.0,
            y: origin.1,
        });
    }
    let edge = |start: i32, len: u32| i32::try_from(len).ok().and_then(|l| start.checked_add(l));
    edge(origin.0, frame.width())
        .zip(edge(origin.1, frame.height()))
        .and_then(|(x1, y1)| RectI::new(origin.0, origin.1, x1, y1).ok())
        .ok_or_else(|| {
            BlurError::InvalidFrame(format!(
                "a {}x{} view at {origin:?} exceeds the coordinate range",
                frame.width(),
                frame.height()
            ))
        })
}

/// Reusable buffers of one `blur` call.
#[derive(Debug, Default)]
struct Scratch {
    /// Blurred values of every box of the current channel, box after box.
    patches: Vec<f32>,
    /// Block means.
    means: Vec<f32>,
    /// Bilinear upsample of the means.
    upsampled: Vec<f32>,
    /// Result of the horizontal gaussian pass.
    horizontal: Vec<f32>,
}

/// A box's padded area in one channel, in full-frame channel coordinates.
#[derive(Debug, Clone, Copy)]
struct Area {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
}

impl Area {
    /// The padded box of `b` at the channel's resolution. The padded box is
    /// even-aligned (or ends at an odd frame edge), so halving it and
    /// rounding the far edges up covers exactly the chroma samples of its
    /// luma pixels.
    fn of(b: &PreparedBox, subsampling: Subsampling) -> Self {
        let s = subsampling.factor();
        let p = b.padded();
        Self {
            x0: p.x0 / s,
            y0: p.y0 / s,
            x1: (p.x1 + s - 1) / s,
            y1: (p.y1 + s - 1) / s,
        }
    }

    fn width(&self) -> usize {
        (self.x1 - self.x0) as usize
    }

    fn height(&self) -> usize {
        (self.y1 - self.y0) as usize
    }

    fn len(&self) -> usize {
        self.width() * self.height()
    }
}

/// The blur of one box in one channel.
struct PatchJob<'a> {
    channel: &'a Channel,
    data: &'a [u8],
    /// Full-frame channel coordinates of the view's first sample.
    offset: (i32, i32),
    area: Area,
    /// 0 = Y, 1 = U, 2 = V.
    index: usize,
    params: &'a PixelParams,
    b: &'a PreparedBox,
}

impl PatchJob<'_> {
    /// The source sample at full-frame channel coordinates `(x, y)`.
    fn read(&self, x: i32, y: i32) -> f32 {
        self.channel.read(
            self.data,
            (x - self.offset.0) as usize,
            (y - self.offset.1) as usize,
        )
    }

    /// Appends the blurred values of the area (row-major) to
    /// `scratch.patches`.
    fn fill(&self, scratch: &mut Scratch) {
        let a = self.area;
        let start = scratch.patches.len();
        scratch.patches.resize(start + a.len(), 0.0);
        let s = self.channel.subsampling.factor() as f32;
        let max = self.channel.max_value();
        match self.params.fill {
            Fill::Smooth { sigma } => {
                let (cols, rows) = self.block_means(scratch);
                let up = &mut scratch.upsampled;
                up.clear();
                up.resize(a.len(), 0.0);
                let wx = bilinear(&cols, a.x0, a.width());
                let wy = bilinear(&rows, a.y0, a.height());
                let nx = cols.len();
                for (y, &(r0, r1, ty)) in wy.iter().enumerate() {
                    for (x, &(c0, c1, tx)) in wx.iter().enumerate() {
                        let m = &scratch.means;
                        let top = lerp(m[r0 * nx + c0], m[r0 * nx + c1], tx);
                        let bottom = lerp(m[r1 * nx + c0], m[r1 * nx + c1], tx);
                        up[y * a.width() + x] = lerp(top, bottom, ty);
                    }
                }
                let kernel = gaussian_kernel(sigma / s);
                let out = &mut scratch.patches[start..];
                separable_gaussian(
                    &scratch.upsampled,
                    &mut scratch.horizontal,
                    out,
                    a.width(),
                    &kernel,
                );
            }
            Fill::Blocks => {
                let (cols, rows) = self.block_means(scratch);
                let cx = cell_of_each(&cols, a.x0, a.width());
                let cy = cell_of_each(&rows, a.y0, a.height());
                let out = &mut scratch.patches[start..];
                for (y, &r) in cy.iter().enumerate() {
                    for (x, &c) in cx.iter().enumerate() {
                        out[y * a.width() + x] = scratch.means[r * cols.len() + c];
                    }
                }
            }
            Fill::BoxMean => {
                let mut sum = 0.0f64;
                for y in a.y0..a.y1 {
                    for x in a.x0..a.x1 {
                        sum += f64::from(self.read(x, y));
                    }
                }
                let mean = (sum / a.len() as f64) as f32;
                scratch.patches[start..].fill(mean);
            }
            Fill::Colour(colour, rgb) => {
                let bits = if max > 255.0 { 10 } else { 8 };
                let value = colour.code_values(rgb, bits)[self.index];
                scratch.patches[start..].fill(value);
            }
        }
        for v in &mut scratch.patches[start..] {
            *v = v.clamp(0.0, max);
        }
    }

    /// Computes the block means of the area into `scratch.means` (row-major,
    /// one row per vertical cell) and returns the column and row cells.
    /// Adds the per-block noise to luma when it is enabled.
    fn block_means(&self, scratch: &mut Scratch) -> (Cells, Cells) {
        let a = self.area;
        let s = self.channel.subsampling.factor() as f32;
        let block = self.b.block / s;
        let cols = Cells::new(self.b.grid_origin.0 as f32 / s, block, a.x0, a.x1);
        let rows = Cells::new(self.b.grid_origin.1 as f32 / s, block, a.y0, a.y1);
        let means = &mut scratch.means;
        means.clear();
        let amplitude = NOISE_AMPLITUDE * self.channel.max_value() / 255.0;
        let noisy = self.params.noise && self.index == 0;
        for r in 0..rows.len() {
            for c in 0..cols.len() {
                let mut sum = 0.0f64;
                for y in rows.bounds[r]..rows.bounds[r + 1] {
                    for x in cols.bounds[c]..cols.bounds[c + 1] {
                        sum += f64::from(self.read(x, y));
                    }
                }
                let count =
                    (rows.bounds[r + 1] - rows.bounds[r]) * (cols.bounds[c + 1] - cols.bounds[c]);
                let mut mean = (sum / f64::from(count)) as f32;
                if noisy {
                    mean += amplitude * noise(self.b.seed, cols.index(c), rows.index(r));
                }
                means.push(mean);
            }
        }
        (cols, rows)
    }
}

/// Cells of the block grid along one axis, cut to `lo..hi`.
#[derive(Debug)]
struct Cells {
    /// Cell boundaries inside `lo..hi`; cell `i` is `bounds[i]..bounds[i + 1]`.
    bounds: Vec<i32>,
    /// Centres of the uncut cells, for bilinear interpolation.
    centres: Vec<f32>,
    /// Grid index of the first cell (cell 0 starts at the grid origin).
    first: i64,
}

impl Cells {
    /// Cells of size `block` with an edge at `origin`, rounded to whole
    /// samples, that overlap `lo..hi`.
    fn new(origin: f32, block: f32, lo: i32, hi: i32) -> Self {
        let (origin, block) = (f64::from(origin), f64::from(block));
        let edge = |k: i64| (origin + k as f64 * block + 0.5).floor() as i32;
        // One cell of slack on each side absorbs the edge rounding.
        let mut k = ((f64::from(lo) - origin) / block).floor() as i64 - 1;
        while edge(k + 1) <= lo {
            k += 1;
        }
        let first = k;
        let mut bounds = vec![lo];
        let mut centres = vec![];
        while edge(k) < hi {
            let (e0, e1) = (edge(k), edge(k + 1));
            bounds.push(e1.min(hi));
            centres.push((e0 + e1) as f32 * 0.5);
            k += 1;
        }
        Self {
            bounds,
            centres,
            first,
        }
    }

    fn len(&self) -> usize {
        self.centres.len()
    }

    /// Grid index of cell `i`.
    fn index(&self, i: usize) -> i64 {
        self.first + i as i64
    }
}

/// For each of `len` samples from `lo`, the two cells to interpolate and
/// the weight of the second one. Samples before the first or after the last
/// cell centre take that cell's value (replicated border).
fn bilinear(cells: &Cells, lo: i32, len: usize) -> Vec<(usize, usize, f32)> {
    let c = &cells.centres;
    let last = c.len() - 1;
    let mut j = 0;
    (0..len)
        .map(|i| {
            let p = (lo + i as i32) as f32 + 0.5;
            while j < last && c[j + 1] <= p {
                j += 1;
            }
            if p <= c[0] {
                (0, 0, 0.0)
            } else if j == last {
                (last, last, 0.0)
            } else {
                (j, j + 1, (p - c[j]) / (c[j + 1] - c[j]))
            }
        })
        .collect()
}

/// For each of `len` samples from `lo`, the index of its cell.
fn cell_of_each(cells: &Cells, lo: i32, len: usize) -> Vec<usize> {
    let mut j = 0;
    (0..len)
        .map(|i| {
            let p = lo + i as i32;
            while cells.bounds[j + 1] <= p {
                j += 1;
            }
            j
        })
        .collect()
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    (b - a).mul_add(t, a)
}

/// Normalised gaussian weights for offsets `-r..=r`, `r = ceil(3 sigma)`.
fn gaussian_kernel(sigma: f32) -> Vec<f32> {
    let radius = (3.0 * sigma).ceil().max(1.0) as i32;
    let weights: Vec<f32> = (-radius..=radius)
        .map(|t| (-((t * t) as f32) / (2.0 * sigma * sigma)).exp())
        .collect();
    let sum: f32 = weights.iter().sum();
    weights.into_iter().map(|w| w / sum).collect()
}

/// Separable gaussian of a `width`-wide image with a replicated border:
/// `src` → `tmp` (horizontal) → `out` (vertical).
fn separable_gaussian(src: &[f32], tmp: &mut Vec<f32>, out: &mut [f32], width: usize, k: &[f32]) {
    let height = src.len() / width;
    let r = (k.len() / 2) as isize;
    let clamp = |i: isize, n: usize| i.clamp(0, n as isize - 1) as usize;
    tmp.clear();
    tmp.resize(src.len(), 0.0);
    for y in 0..height {
        let row = &src[y * width..(y + 1) * width];
        for x in 0..width {
            tmp[y * width + x] = k
                .iter()
                .enumerate()
                .map(|(t, w)| w * row[clamp(x as isize + t as isize - r, width)])
                .sum();
        }
    }
    for y in 0..height {
        for x in 0..width {
            out[y * width + x] = k
                .iter()
                .enumerate()
                .map(|(t, w)| w * tmp[clamp(y as isize + t as isize - r, height) * width + x])
                .sum();
        }
    }
}

/// Deterministic noise in `-1.0..1.0` for block `(kx, ky)` of a box with
/// `seed`. Blocks are indexed from the grid origin, so the noise moves with
/// the text and stays constant over a track.
fn noise(seed: u32, kx: i64, ky: i64) -> f32 {
    let mut z = u64::from(seed)
        ^ (kx as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (ky as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    ((z >> 40) as f32 / (1u32 << 23) as f32) - 1.0
}

/// Mixes a box's blurred values into the plane through its mask.
fn composite(
    channel: &Channel,
    data: &mut [u8],
    offset: (i32, i32),
    area: Area,
    b: &PreparedBox,
    patch: &[f32],
) {
    let max = channel.max_value();
    let luma = channel.subsampling == Subsampling::Luma;
    for y in area.y0..area.y1 {
        for x in area.x0..area.x1 {
            let m = if luma {
                b.coverage(x, y)
            } else {
                b.chroma_coverage(x, y)
            };
            if m <= 0.0 {
                continue;
            }
            let (px, py) = ((x - offset.0) as usize, (y - offset.1) as usize);
            let blurred = patch[(y - area.y0) as usize * area.width() + (x - area.x0) as usize];
            let src = channel.read(data, px, py);
            let value = (blurred - src).mul_add(m, src).round().clamp(0.0, max);
            channel.write(data, px, py, value as u16);
        }
    }
}

#[cfg(test)]
mod tests {
    use blurrd_core::{BlurBox, BlurParams, BlurStyle, RectI, Region, SolidColor};

    use super::CpuBlur;
    use crate::{
        BlurBackend, BlurError, PixelParams, PlaneMut, YuvColour, YuvFrameMut, prepare_boxes,
    };

    /// An NV12 frame with a diagonal stripe pattern.
    fn nv12(w: u32, h: u32) -> (Vec<u8>, Vec<u8>) {
        let y = (0..w * h)
            .map(|i| ((i % w) * 7 + (i / w) * 13) as u8)
            .collect();
        let uv = (0..w.div_ceil(2) * h.div_ceil(2) * 2)
            .map(|i| (i * 3) as u8)
            .collect();
        (y, uv)
    }

    fn frame<'a>(y: &'a mut [u8], uv: &'a mut [u8], w: u32, h: u32) -> YuvFrameMut<'a> {
        YuvFrameMut::Nv12 {
            y: PlaneMut {
                data: y,
                stride: w as usize,
                width: w,
                height: h,
            },
            uv: PlaneMut {
                data: uv,
                stride: 2 * w.div_ceil(2) as usize,
                width: w.div_ceil(2),
                height: h.div_ceil(2),
            },
        }
    }

    fn blur_box(rect: RectI) -> BlurBox {
        BlurBox {
            rect,
            region: Region::KillFeed,
            seed: 1,
            grid_origin: (rect.x0, rect.y0),
            block_scale: 1.0,
        }
    }

    fn run(
        params: &BlurParams,
        rect: RectI,
        w: u32,
        h: u32,
    ) -> (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) {
        let p = PixelParams::new(params, 1.0, YuvColour::default()).unwrap();
        let mut boxes = vec![];
        prepare_boxes(&[blur_box(rect)], w, h, &p, &mut boxes);
        let (y0, uv0) = nv12(w, h);
        let (mut y, mut uv) = (y0.clone(), uv0.clone());
        CpuBlur
            .blur(&mut frame(&mut y, &mut uv, w, h), (0, 0), &boxes, &p)
            .unwrap();
        (y0, uv0, y, uv)
    }

    #[test]
    fn writes_only_inside_the_padded_box() {
        let rect = RectI::new(20, 10, 60, 24).unwrap();
        let (y0, _, y, _) = run(&BlurParams::default(), rect, 96, 40);
        // Padded box with the default padding (4, 3): [16, 6, 64, 28].
        for row in 0..40 {
            for col in 0..96 {
                let i = row * 96 + col;
                if !(16..64).contains(&col) || !(6..28).contains(&row) {
                    assert_eq!(
                        y[i], y0[i],
                        "pixel ({col}, {row}) outside the padded box changed"
                    );
                }
            }
        }
        assert_ne!(y[15 * 96 + 40], y0[15 * 96 + 40], "the core is blurred");
    }

    #[test]
    fn solid_colour_fills_the_core_exactly() {
        let params = BlurParams {
            style: BlurStyle::Solid {
                color: SolidColor::Rgb([255, 255, 255]),
            },
            ..BlurParams::default()
        };
        let rect = RectI::new(21, 11, 59, 23).unwrap();
        let (_, _, y, uv) = run(&params, rect, 96, 40);
        for row in 11..23 {
            for col in 21..59 {
                assert_eq!(y[row * 96 + col], 235);
            }
        }
        // Chroma pixels touching the core (luma columns 20..60, rows 10..24).
        for row in 5..12 {
            for col in 10..30 {
                assert_eq!(uv[row * 96 + 2 * col], 128);
                assert_eq!(uv[row * 96 + 2 * col + 1], 128);
            }
        }
    }

    #[test]
    fn blurs_p010_natively() {
        let (w, h) = (64u32, 32u32);
        let mut luma: Vec<u8> = (0..w * h)
            .flat_map(|i| ((((i % w) * 37) % 1024) as u16 * 64).to_le_bytes())
            .collect();
        let mut chroma = vec![0u8; (w * h) as usize];
        for pair in chroma.as_chunks_mut::<2>().0 {
            pair.copy_from_slice(&(512u16 << 6).to_le_bytes());
        }
        let params = PixelParams::new(&BlurParams::default(), 1.0, YuvColour::default()).unwrap();
        let mut boxes = vec![];
        let rect = RectI::new(10, 8, 50, 22).unwrap();
        prepare_boxes(&[blur_box(rect)], w, h, &params, &mut boxes);
        let before = luma.clone();
        let mut frame = YuvFrameMut::P010 {
            y: PlaneMut {
                data: &mut luma,
                stride: 2 * w as usize,
                width: w,
                height: h,
            },
            uv: PlaneMut {
                data: &mut chroma,
                stride: 2 * w as usize,
                width: w / 2,
                height: h / 2,
            },
        };
        CpuBlur.blur(&mut frame, (0, 0), &boxes, &params).unwrap();
        assert_ne!(luma, before);
        for pair in luma.as_chunks::<2>().0 {
            assert_eq!(u16::from_le_bytes(*pair) & 0x3f, 0, "low 6 bits stay zero");
        }
    }

    #[test]
    fn rejects_bad_views_without_writing() {
        let (w, h) = (64u32, 32u32);
        let p = PixelParams::new(&BlurParams::default(), 1.0, YuvColour::default()).unwrap();
        let mut boxes = vec![];
        prepare_boxes(
            &[blur_box(RectI::new(10, 8, 50, 22).unwrap())],
            w,
            h,
            &p,
            &mut boxes,
        );
        let (y0, uv0) = nv12(w, h);
        let (mut y, mut uv) = (y0.clone(), uv0.clone());
        let result = CpuBlur.blur(&mut frame(&mut y, &mut uv, w, h), (1, 0), &boxes, &p);
        assert_eq!(result, Err(BlurError::OddOrigin { x: 1, y: 0 }));
        let result = CpuBlur.blur(&mut frame(&mut y, &mut uv, w, h), (8, 0), &boxes, &p);
        assert!(matches!(result, Err(BlurError::BoxOutsideView { .. })));
        let mut short = vec![0u8; 10];
        let result = CpuBlur.blur(&mut frame(&mut short, &mut uv, w, h), (0, 0), &boxes, &p);
        assert!(matches!(result, Err(BlurError::InvalidFrame(_))));
        assert_eq!((y, uv), (y0, uv0));
    }
}
