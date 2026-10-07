//! The CPU blur backend.

#![allow(
    clippy::cast_possible_wrap,
    reason = "box and plane sizes come from i32 frame coordinates, so index arithmetic cannot wrap"
)]

use std::cell::RefCell;

use blurrd_core::RectI;
use wide::f32x8;

use crate::frame::{
    Channel, Lanes, Packed8, Packed10, Paired8, Paired10, SampleFormat, Subsampling,
};
use crate::params::{Fill, NOISE_AMPLITUDE};
use crate::prepare::AxisMask;
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
/// The smooth fill exploits that upsampling and the gaussian are linear and
/// separable: each row of block means is upsampled and blurred horizontally
/// once, and every output row is a weighted sum of those few rows, with the
/// vertical upsample folded into the vertical gaussian weights. That is the
/// same result as upsampling the whole box first, at a fraction of the
/// work. The inner loops run on 8-lane vectors.
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
        SCRATCH.with_borrow_mut(|scratch| blur_boxes(frame, origin, boxes, params, scratch));
        Ok(())
    }
}

thread_local! {
    /// Buffers reused by every `blur` call on a thread. Allocating them per
    /// call costs more than the blur itself: large fresh allocations come
    /// straight from the operating system and fault in page by page.
    static SCRATCH: RefCell<Scratch> = RefCell::new(Scratch::default());
}

/// Blurs validated `boxes` into `frame` with the given buffers.
fn blur_boxes(
    frame: &mut YuvFrameMut<'_>,
    origin: (i32, i32),
    boxes: &[PreparedBox],
    params: &PixelParams,
    scratch: &mut Scratch,
) {
    let mut kernels = std::mem::take(&mut scratch.kernels);
    let mut geometries = std::mem::take(&mut scratch.geometries);
    geometries.resize_with(boxes.len(), Default::default);
    // Mask terms of every box at luma and at chroma resolution, shared by
    // the channels.
    let mut masks = std::mem::take(&mut scratch.masks);
    masks.resize_with(boxes.len(), Default::default);
    for (b, [luma, chroma]) in boxes.iter().zip(masks.iter_mut()) {
        luma.build(b, Subsampling::Luma);
        chroma.build(b, Subsampling::Chroma);
    }
    for (index, channel) in frame.channels().into_iter().enumerate() {
        let s = channel.subsampling.factor();
        let offset = (origin.0 / s, origin.1 / s);
        scratch.patches.clear();
        // Grid cells and interpolation weights depend only on the box and
        // the resolution: U reuses them for V.
        let resolution = usize::from(channel.subsampling == Subsampling::Chroma);
        if index < 2 {
            for (b, geometry) in boxes.iter().zip(geometries.iter_mut()) {
                let kernel = match params.fill {
                    Fill::Smooth { sigma_per_block } => {
                        kernels.get(sigma_per_block * b.block / s as f32)
                    }
                    Fill::Blocks | Fill::BoxMean | Fill::Colour(..) => &[],
                };
                geometry[resolution].build(b, channel.subsampling, kernel);
            }
        }
        let source: &[u8] = frame.plane_data(channel.plane);
        for (b, geometry) in boxes.iter().zip(&geometries) {
            let job = PatchJob {
                geometry: &geometry[resolution],
                channel: &channel,
                data: source,
                offset,
                area: Area::of(b, channel.subsampling),
                index,
                params,
                b,
            };
            job.fill(scratch);
        }
        let target = frame.plane_data(channel.plane);
        let mut start = 0;
        for (b, [luma, chroma]) in boxes.iter().zip(&masks) {
            let area = Area::of(b, channel.subsampling);
            let end = start + area.len();
            let mask = if channel.subsampling == Subsampling::Luma {
                luma
            } else {
                chroma
            };
            let patch = &scratch.patches[start..end];
            composite(
                &channel,
                target,
                offset,
                area,
                b,
                mask,
                patch,
                &mut scratch.row_mask,
            );
            start = end;
        }
    }
    scratch.masks = masks;
    scratch.geometries = geometries;
    scratch.kernels = kernels;
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

/// Buffers of a `blur` call, reused across calls.
#[derive(Debug, Default)]
struct Scratch {
    /// Mask terms of every box at luma and at chroma resolution.
    masks: Vec<[ChannelMask; 2]>,
    /// Blurred values of every box of the current channel, box after box.
    patches: Vec<f32>,
    /// Gaussian kernels by sigma.
    kernels: Kernels,
    /// Grid cells and interpolation weights of every box at luma and at
    /// chroma resolution.
    geometries: Vec<[Geometry; 2]>,
    /// Block means, one row per vertical cell.
    means: Vec<f32>,
    /// Column sums of the current cell row.
    sums: Vec<f32>,
    /// Horizontally upsampled and blurred rows of means.
    smooth_rows: Vec<f32>,
    /// One upsampled row of means with its replicated border.
    padded_row: Vec<f32>,
    /// Mask of one row in a corner band.
    row_mask: Vec<f32>,
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
    /// Grid cells and interpolation weights of the box at this resolution.
    geometry: &'a Geometry,
    b: &'a PreparedBox,
}

impl PatchJob<'_> {
    /// Sums the source samples of full-frame channel rows `y0..y1` from
    /// column `x` per column into `sums`.
    fn sum_band(&self, x: i32, (y0, y1): (i32, i32), sums: &mut [f32]) {
        let column = (x - self.offset.0) as usize;
        let (first, step) = self
            .channel
            .row_start(column, (y0 - self.offset.1) as usize);
        let stride = self
            .channel
            .row_start(column, (y0 + 1 - self.offset.1) as usize)
            .0
            - first;
        let rows = (y1 - y0) as usize;
        let band = Band {
            data: self.data,
            first,
            stride,
            rows,
        };
        match (self.channel.format, step) {
            (SampleFormat::U8, 1) => band.sum::<Packed8>(sums),
            (SampleFormat::U8, _) => band.sum::<Paired8>(sums),
            (SampleFormat::U16Le, 2) => band.sum::<Packed10>(sums),
            (SampleFormat::U16Le, _) => band.sum::<Paired10>(sums),
        }
    }

    /// Appends the blurred values of the area (row-major) to
    /// `scratch.patches`. Noise can push them slightly out of range; the
    /// composite clamps.
    fn fill(&self, scratch: &mut Scratch) {
        let a = self.area;
        let start = scratch.patches.len();
        scratch.patches.resize(start + a.len(), 0.0);
        let max = self.channel.max_value();
        match self.params.fill {
            Fill::Smooth { .. } => {
                self.block_means(scratch);
                smooth(scratch, self.geometry, start, a);
            }
            Fill::Blocks => {
                self.block_means(scratch);
                let (cols, rows) = (&self.geometry.cols, &self.geometry.rows);
                let out = &mut scratch.patches[start..];
                let w = a.width();
                for r in 0..rows.len() {
                    let y0 = (rows.bounds[r] - a.y0) as usize;
                    let y1 = (rows.bounds[r + 1] - a.y0) as usize;
                    let first = &mut out[y0 * w..(y0 + 1) * w];
                    for c in 0..cols.len() {
                        let x0 = (cols.bounds[c] - a.x0) as usize;
                        let x1 = (cols.bounds[c + 1] - a.x0) as usize;
                        first[x0..x1].fill(scratch.means[r * cols.len() + c]);
                    }
                    let (head, tail) = out.split_at_mut((y0 + 1) * w);
                    let first = &head[y0 * w..];
                    for row in tail[..(y1 - y0 - 1) * w].chunks_exact_mut(w) {
                        row.copy_from_slice(first);
                    }
                }
            }
            Fill::BoxMean => {
                scratch.sums.resize(a.width(), 0.0);
                self.sum_band(a.x0, (a.y0, a.y1), &mut scratch.sums);
                let mean = lane_sum(&scratch.sums) / a.len() as f32;
                scratch.patches[start..].fill(mean);
            }
            Fill::Colour(colour, rgb) => {
                let bits = if max > 255.0 { 10 } else { 8 };
                let value = colour.code_values(rgb, bits)[self.index];
                scratch.patches[start..].fill(value);
            }
        }
    }

    /// Computes the block means of the area into `scratch.means`
    /// (row-major, one row per vertical cell). Adds the per-block
    /// noise to luma when it is enabled.
    fn block_means(&self, scratch: &mut Scratch) {
        let a = self.area;
        let (cols, rows) = (&self.geometry.cols, &self.geometry.rows);
        let w = a.width();
        scratch.means.clear();
        scratch.sums.resize(w, 0.0);
        let amplitude = NOISE_AMPLITUDE * self.channel.max_value() / 255.0;
        let noisy = self.params.noise && self.index == 0;
        for r in 0..rows.len() {
            self.sum_band(
                a.x0,
                (rows.bounds[r], rows.bounds[r + 1]),
                &mut scratch.sums,
            );
            let height = (rows.bounds[r + 1] - rows.bounds[r]) as f32;
            for c in 0..cols.len() {
                let x0 = (cols.bounds[c] - a.x0) as usize;
                let x1 = (cols.bounds[c + 1] - a.x0) as usize;
                let sum: f32 = scratch.sums[x0..x1].iter().sum();
                let mut mean = sum / (height * (x1 - x0) as f32);
                if noisy {
                    mean += amplitude * noise(self.b.seed, cols.index(c), rows.index(r));
                }
                scratch.means.push(mean);
            }
        }
    }
}

/// A box's block grid and interpolation weights at one resolution.
#[derive(Debug, Default)]
struct Geometry {
    /// Grid cells along x and y.
    cols: Cells,
    rows: Cells,
    /// Horizontal bilinear taps per output column: the two cells and the
    /// weight of the second.
    tap_first: Vec<u32>,
    tap_second: Vec<u32>,
    tap_weight: Vec<f32>,
    /// Gaussian weights for the horizontal pass.
    kernel: Vec<f32>,
    /// Folded vertical weights, `rows.len()` per output row.
    vertical: Vec<f32>,
    /// Per output row, the first and last cell row with a non-zero weight.
    bands: Vec<(usize, usize)>,
}

impl Geometry {
    /// Computes the grid of box `b` at the given resolution and, when
    /// `kernel` is not empty (smooth fill), the interpolation weights.
    fn build(&mut self, b: &PreparedBox, subsampling: Subsampling, kernel: &[f32]) {
        let area = Area::of(b, subsampling);
        let s = subsampling.factor() as f32;
        let block = b.block / s;
        let (gx, gy) = b.grid_origin;
        self.cols.compute(gx as f32 / s, block, area.x0, area.x1);
        self.rows.compute(gy as f32 / s, block, area.y0, area.y1);
        self.kernel.clear();
        self.kernel.extend_from_slice(kernel);
        if kernel.is_empty() {
            return;
        }
        let mut taps = Vec::new();
        bilinear_taps(&self.cols, area.x0, area.width(), &mut taps);
        self.tap_first.clear();
        self.tap_second.clear();
        self.tap_weight.clear();
        for &(c0, c1, t) in &taps {
            self.tap_first.push(c0 as u32);
            self.tap_second.push(c1 as u32);
            self.tap_weight.push(t);
        }

        // A = G * U: the vertical bilinear weights folded into the gaussian
        // (replicated border), one row of cell weights per output row.
        let (height, ny) = (area.height(), self.rows.len());
        let radius = kernel.len() / 2;
        bilinear_taps(&self.rows, area.y0, height, &mut taps);
        self.vertical.clear();
        self.vertical.resize(height * ny, 0.0);
        self.bands.clear();
        for (y, weights) in self.vertical.chunks_exact_mut(ny).enumerate() {
            // Consecutive taps mostly share their pair of cells: sum them in
            // registers and write when the pair changes.
            let (mut pair, mut sum0, mut sum1) = ((0, 0), 0.0f32, 0.0f32);
            for (t, &k) in kernel.iter().enumerate() {
                let source = (y + t).saturating_sub(radius).min(height - 1);
                let (r0, r1, w) = taps[source];
                if (r0, r1) != pair {
                    weights[pair.0] += sum0;
                    weights[pair.1] += sum1;
                    (pair, sum0, sum1) = ((r0, r1), 0.0, 0.0);
                }
                sum0 += k * (1.0 - w);
                sum1 += k * w;
            }
            weights[pair.0] += sum0;
            weights[pair.1] += sum1;
            let first = weights.iter().position(|&w| w != 0.0).unwrap_or(0);
            let last = weights.iter().rposition(|&w| w != 0.0).unwrap_or(0);
            self.bands.push((first, last));
        }
    }
}

/// Bilinear upsample of the block means followed by a separable gaussian
/// with a replicated border, written to `scratch.patches[start..]`.
///
/// Horizontal pass: each row of means is upsampled to the area's width,
/// padded by the kernel radius with its edge values and convolved. Vertical
/// pass: output row `y` is `sum_k A[y][k] * row_k` with the folded weights
/// of [`Geometry`]. That equals upsampling the whole area first, because
/// both passes are linear and act on different axes.
fn smooth(scratch: &mut Scratch, geometry: &Geometry, start: usize, area: Area) {
    let (width, height) = (area.width(), area.height());
    let (nx, ny) = (geometry.cols.len(), geometry.rows.len());
    let kernel = &geometry.kernel;
    let radius = kernel.len() / 2;

    scratch.smooth_rows.clear();
    scratch.smooth_rows.resize(ny * width, 0.0);
    scratch.padded_row.resize(width + 2 * radius, 0.0);
    for r in 0..ny {
        let means = &scratch.means[r * nx..(r + 1) * nx];
        let padded = &mut scratch.padded_row;
        upsample(means, geometry, &mut padded[radius..radius + width]);
        let (left, right) = (padded[radius], padded[radius + width - 1]);
        padded[..radius].fill(left);
        padded[radius + width..].fill(right);
        let out = &mut scratch.smooth_rows[r * width..(r + 1) * width];
        convolve(padded, kernel, out);
    }

    let out = &mut scratch.patches[start..start + width * height];
    for ((row, weights), &(first, last)) in out
        .chunks_exact_mut(width)
        .zip(geometry.vertical.chunks_exact(ny))
        .zip(&geometry.bands)
    {
        // Only a few cell rows reach an output row; skip the zero weights.
        let rows = &scratch.smooth_rows[first * width..(last + 1) * width];
        weighted_rows(&weights[first..=last], rows, row);
    }
}

/// `out[x] = sum_t kernel[t] * padded[x + t]`. Four groups of eight
/// outputs are accumulated at once, so the additions of one group do not
/// wait for each other.
fn convolve(padded: &[f32], kernel: &[f32], out: &mut [f32]) {
    let (chunks, rest) = out.as_chunks_mut::<8>();
    let (blocks, remainder) = chunks.as_chunks_mut::<4>();
    let mut base = 0;
    for block in blocks {
        let mut acc = [f32x8::ZERO; 4];
        for (t, &k) in kernel.iter().enumerate() {
            let k = f32x8::splat(k);
            let (window, _) = padded[base + t..base + t + 32].as_chunks::<8>();
            for (a, s) in acc.iter_mut().zip(window) {
                *a += k * f32x8::from(*s);
            }
        }
        for (o, a) in block.iter_mut().zip(acc) {
            *o = a.to_array();
        }
        base += 32;
    }
    for o in remainder {
        let mut acc = f32x8::ZERO;
        for (t, &k) in kernel.iter().enumerate() {
            acc += f32x8::splat(k) * load8(&padded[base + t..]);
        }
        *o = acc.to_array();
        base += 8;
    }
    for (i, o) in rest.iter_mut().enumerate() {
        *o = kernel
            .iter()
            .enumerate()
            .map(|(t, &k)| k * padded[base + i + t])
            .sum();
    }
}

/// Bilinear upsample of one row of means with the horizontal taps, eight
/// samples at a time.
fn upsample(means: &[f32], geometry: &Geometry, out: &mut [f32]) {
    let (first, second, weight) = (
        &geometry.tap_first,
        &geometry.tap_second,
        &geometry.tap_weight,
    );
    let (chunks, rest) = out.as_chunks_mut::<8>();
    let (first_chunks, _) = first.as_chunks::<8>();
    let (second_chunks, _) = second.as_chunks::<8>();
    let (weight_chunks, _) = weight.as_chunks::<8>();
    for (((o, i0), i1), t) in chunks
        .iter_mut()
        .zip(first_chunks)
        .zip(second_chunks)
        .zip(weight_chunks)
    {
        let m0 = f32x8::from(i0.map(|i| means[i as usize]));
        let m1 = f32x8::from(i1.map(|i| means[i as usize]));
        *o = ((m1 - m0) * f32x8::from(*t) + m0).to_array();
    }
    let done = chunks.len() * 8;
    for (i, o) in rest.iter_mut().enumerate() {
        let k = done + i;
        *o = lerp(
            means[first[k] as usize],
            means[second[k] as usize],
            weight[k],
        );
    }
}

/// `out[x] = sum_r weights[r] * rows[r * out.len() + x]`, four groups of
/// eight outputs at a time.
fn weighted_rows(weights: &[f32], rows: &[f32], out: &mut [f32]) {
    let width = out.len();
    let (chunks, rest) = out.as_chunks_mut::<8>();
    let (blocks, remainder) = chunks.as_chunks_mut::<4>();
    let mut base = 0;
    for block in blocks {
        let mut acc = [f32x8::ZERO; 4];
        for (r, &w) in weights.iter().enumerate() {
            let w = f32x8::splat(w);
            let start = r * width + base;
            let (window, _) = rows[start..start + 32].as_chunks::<8>();
            for (a, s) in acc.iter_mut().zip(window) {
                *a += w * f32x8::from(*s);
            }
        }
        for (o, a) in block.iter_mut().zip(acc) {
            *o = a.to_array();
        }
        base += 32;
    }
    for o in remainder {
        let mut acc = f32x8::ZERO;
        for (r, &w) in weights.iter().enumerate() {
            acc += f32x8::splat(w) * load8(&rows[r * width + base..]);
        }
        *o = acc.to_array();
        base += 8;
    }
    for (i, o) in rest.iter_mut().enumerate() {
        *o = weights
            .iter()
            .enumerate()
            .map(|(r, &w)| w * rows[r * width + base + i])
            .sum();
    }
}

/// The first eight values of `values` as a vector.
fn load8(values: &[f32]) -> f32x8 {
    let mut lanes = [0.0; 8];
    lanes.copy_from_slice(&values[..8]);
    f32x8::from(lanes)
}

/// Consecutive rows of one channel inside a plane.
struct Band<'a> {
    data: &'a [u8],
    /// Byte index of the first sample of the first row.
    first: usize,
    /// Bytes from one row to the next.
    stride: usize,
    rows: usize,
}

impl Band<'_> {
    /// `sums[i] = sum over the rows of sample i`. Four groups of eight
    /// columns are summed at once, so the additions of one group do not
    /// wait for each other.
    fn sum<L: Lanes>(&self, sums: &mut [f32]) {
        let (chunks, rest) = sums.as_chunks_mut::<8>();
        let (blocks, remainder) = chunks.as_chunks_mut::<4>();
        let mut column = 0;
        for block in blocks {
            let mut acc = [f32x8::ZERO; 4];
            for row in 0..self.rows {
                let start = self.first + row * self.stride + column * L::STEP;
                for (k, a) in acc.iter_mut().enumerate() {
                    *a += L::load8(&self.data[start + k * 8 * L::STEP..]);
                }
            }
            for (o, a) in block.iter_mut().zip(acc) {
                *o = a.to_array();
            }
            column += 32;
        }
        for o in remainder {
            let mut acc = f32x8::ZERO;
            for row in 0..self.rows {
                acc += L::load8(&self.data[self.first + row * self.stride + column * L::STEP..]);
            }
            *o = acc.to_array();
            column += 8;
        }
        for (i, o) in rest.iter_mut().enumerate() {
            *o = (0..self.rows)
                .map(|row| {
                    L::load(
                        self.data,
                        self.first + row * self.stride + (column + i) * L::STEP,
                    )
                })
                .sum();
        }
    }
}

/// Sum of the values, accumulated in eight lanes.
fn lane_sum(values: &[f32]) -> f32 {
    let (chunks, rest) = values.as_chunks::<8>();
    let lanes = chunks
        .iter()
        .fold(f32x8::ZERO, |acc, c| acc + f32x8::from(*c));
    lanes.reduce_add() + rest.iter().sum::<f32>()
}

/// Cells of the block grid along one axis, cut to the area.
#[derive(Debug, Default)]
struct Cells {
    /// Cell boundaries inside the area; cell `i` is `bounds[i]..bounds[i + 1]`.
    bounds: Vec<i32>,
    /// Centres of the uncut cells, for bilinear interpolation.
    centres: Vec<f32>,
    /// Grid index of the first cell (cell 0 starts at the grid origin).
    first: i64,
}

impl Cells {
    /// Cells of size `block` with an edge at `origin`, rounded to whole
    /// samples, that overlap `lo..hi`.
    fn compute(&mut self, origin: f32, block: f32, lo: i32, hi: i32) {
        let (origin, block) = (f64::from(origin), f64::from(block));
        let edge = |k: i64| (origin + k as f64 * block + 0.5).floor() as i32;
        // One cell of slack on each side absorbs the edge rounding.
        let mut k = ((f64::from(lo) - origin) / block).floor() as i64 - 1;
        while edge(k + 1) <= lo {
            k += 1;
        }
        self.first = k;
        self.bounds.clear();
        self.bounds.push(lo);
        self.centres.clear();
        while edge(k) < hi {
            let (e0, e1) = (edge(k), edge(k + 1));
            self.bounds.push(e1.min(hi));
            self.centres.push((e0 + e1) as f32 * 0.5);
            k += 1;
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
/// the weight of the second one, written to `out`. Samples before the first
/// or after the last cell centre take that cell's value (replicated
/// border).
fn bilinear_taps(cells: &Cells, lo: i32, len: usize, out: &mut Vec<(usize, usize, f32)>) {
    let c = &cells.centres;
    let last = c.len() - 1;
    let mut j = 0;
    out.clear();
    out.extend((0..len).map(|i| {
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
    }));
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    (b - a) * t + a
}

/// Gaussian kernels, built once per sigma and kept across calls. The boxes
/// of one video use a few block sizes (two block scales, luma and chroma);
/// the cache is emptied when it grows beyond that, for example after a
/// switch to a video with another HUD scale.
#[derive(Debug, Default)]
struct Kernels(Vec<(u32, Vec<f32>)>);

impl Kernels {
    /// Most kernels kept.
    const CAPACITY: usize = 8;

    /// The kernel for `sigma` (in samples of the channel).
    fn get(&mut self, sigma: f32) -> &[f32] {
        let key = sigma.to_bits();
        if self.0.len() >= Self::CAPACITY && self.0.iter().all(|(k, _)| *k != key) {
            self.0.clear();
        }
        let index = self
            .0
            .iter()
            .position(|(k, _)| *k == key)
            .unwrap_or_else(|| {
                self.0.push((key, gaussian_kernel(sigma)));
                self.0.len() - 1
            });
        &self.0[index].1
    }
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

/// A box's mask terms at the resolution of one channel.
///
/// Each sample covers one luma column and row (luma) or two of each
/// (chroma); `columns` and `rows` hold their terms, `pooled` the max-pooled
/// column masks and `corners` the samples in a corner column.
#[derive(Debug, Default)]
struct ChannelMask {
    columns: Vec<[AxisMask; 2]>,
    rows: Vec<[AxisMask; 2]>,
    pooled: Vec<f32>,
    corners: Vec<usize>,
    /// Whether this is the chroma mask (two luma pixels per axis).
    chroma: bool,
}

impl ChannelMask {
    /// Computes the terms of box `b` at the given resolution.
    fn build(&mut self, b: &PreparedBox, subsampling: Subsampling) {
        let area = Area::of(b, subsampling);
        let factor = subsampling.factor();
        self.chroma = factor == 2;
        // Luma terms of the samples' columns and rows; a second luma
        // column or row outside the padded box has mask 0.
        let terms = |at: fn(&PreparedBox, i32) -> AxisMask, v: i32, end: i32| {
            let first = at(b, factor * v);
            let second = if factor == 1 {
                first
            } else if 2 * v + 1 < end {
                at(b, 2 * v + 1)
            } else {
                AxisMask::OUTSIDE
            };
            [first, second]
        };
        let padded = b.padded();
        self.columns.clear();
        self.columns
            .extend((area.x0..area.x1).map(|x| terms(PreparedBox::column, x, padded.x1)));
        self.rows.clear();
        self.rows
            .extend((area.y0..area.y1).map(|y| terms(PreparedBox::row, y, padded.y1)));
        self.pooled.clear();
        self.pooled
            .extend(self.columns.iter().map(|[c0, c1]| c0.mask.max(c1.mask)));
        self.corners.clear();
        self.corners.extend(
            self.columns
                .iter()
                .enumerate()
                .filter(|(_, [c0, c1])| c0.corner || c1.corner)
                .map(|(i, _)| i),
        );
    }
}

/// Mixes a box's blurred values into the plane through its mask.
///
/// The mask is the minimum of a column and a row term everywhere except in
/// the rounded corners (see [`PreparedBox::pixel_mask`]), so rows outside
/// the corner bands use one row value against the precomputed column
/// profile. Chroma takes the maximum over the 2×2 luma pixels of each
/// sample; outside the corners that is the minimum of the max-pooled
/// column and row terms.
#[allow(
    clippy::too_many_arguments,
    reason = "internal hot path; a struct would only rename them"
)]
fn composite(
    channel: &Channel,
    data: &mut [u8],
    offset: (i32, i32),
    area: Area,
    b: &PreparedBox,
    mask: &ChannelMask,
    patch: &[f32],
    row_mask: &mut Vec<f32>,
) {
    let w = area.width();
    let max = channel.max_value();
    let px = (area.x0 - offset.0) as usize;
    let mix_row = match (channel.format, channel.row_start(0, 0).1) {
        (SampleFormat::U8, 1) => mix_row::<Packed8>,
        (SampleFormat::U8, _) => mix_row::<Paired8>,
        (SampleFormat::U16Le, 2) => mix_row::<Packed10>,
        (SampleFormat::U16Le, _) => mix_row::<Paired10>,
    };
    for ((y, blurred), &[r0, r1]) in (area.y0..area.y1)
        .zip(patch.chunks_exact(w))
        .zip(&mask.rows)
    {
        let (start, _) = channel.row_start(px, (y - offset.1) as usize);
        let pooled_row = r0.mask.max(r1.mask);
        if r0.corner || r1.corner {
            // Full mask for this row: separable terms, then the corners.
            row_mask.clear();
            row_mask.extend(mask.pooled.iter().map(|&m| m.min(pooled_row)));
            for &i in &mask.corners {
                let [c0, c1] = mask.columns[i];
                row_mask[i] = if mask.chroma {
                    b.pixel_mask(c0, r0)
                        .max(b.pixel_mask(c1, r0))
                        .max(b.pixel_mask(c0, r1))
                        .max(b.pixel_mask(c1, r1))
                } else {
                    b.pixel_mask(c0, r0)
                };
            }
            mix_row(data, start, blurred, row_mask, 1.0, max);
        } else {
            mix_row(data, start, blurred, &mask.pooled, pooled_row, max);
        }
    }
}

/// Mixes one row in place: sample `i` (from byte `start`) becomes
/// `round(clamp(src + (blurred - src) * min(mask[i], row_mask)))`, eight
/// lanes at a time.
fn mix_row<L: Lanes>(
    data: &mut [u8],
    start: usize,
    blurred: &[f32],
    mask: &[f32],
    row_mask: f32,
    max: f32,
) {
    let (zero, half) = (f32x8::ZERO, f32x8::splat(0.5));
    let (row_v, max_v) = (f32x8::splat(row_mask), f32x8::splat(max));
    let (blur_chunks, blur_rest) = blurred.as_chunks::<8>();
    let (mask_chunks, mask_rest) = mask[..blurred.len()].as_chunks::<8>();
    for (n, (b, m)) in blur_chunks.iter().zip(mask_chunks).enumerate() {
        let chunk = &mut data[start + n * 8 * L::STEP..][..L::SPAN];
        let s = L::load8(chunk);
        let (b, m) = (f32x8::from(*b), f32x8::from(*m).fast_min(row_v));
        // All values are finite, so the fast (not NaN-aware) operations are
        // exact here.
        let mixed = ((b - s) * m + s).fast_max(zero).fast_min(max_v) + half;
        L::store8(chunk, mixed.fast_trunc_int());
    }
    let rest_start = start + blur_chunks.len() * 8 * L::STEP;
    for (k, (&b, &m)) in blur_rest.iter().zip(mask_rest).enumerate() {
        let i = rest_start + k * L::STEP;
        let s = L::load(data, i);
        let mixed = ((b - s) * m.min(row_mask) + s).clamp(0.0, max) + 0.5;
        L::store(data, i, mixed as i32);
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
