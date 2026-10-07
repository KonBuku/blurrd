//! The CPU blur backend.

#![allow(
    clippy::cast_possible_wrap,
    reason = "box and plane sizes come from i32 frame coordinates, so index arithmetic cannot wrap"
)]

use std::cell::RefCell;

use blurrd_core::RectI;
use wide::f32x8;

use crate::frame::{Layout, LayoutKind, Packed8, Packed10, Pair8, Pair10, Subsampling, Target};
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
/// work. The output rows are produced one at a time and mixed into the
/// plane right away. The inner loops run on 8-lane vectors, and the
/// interleaved chroma of NV12 and P010 is read and written as U/V pairs in
/// one pass.
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

/// Buffers of a `blur` call, reused across calls.
#[derive(Debug, Default)]
struct Scratch {
    /// The masks and grids of the box shapes seen so far.
    shapes: Shapes,
    /// Intermediate values of the target being blurred.
    work: Work,
}

/// The mask terms and block grids of the boxes, kept across calls.
///
/// Both depend only on a box's shape relative to its padded box (see
/// [`PreparedBox::shape_key`]) and on the fill, and are computed in
/// coordinates relative to the padded box. Boxes of the same size share
/// them within a frame, and a name that stays or moves by whole pixels
/// reuses them from the frames before, as kill-feed and scoreboard entries
/// do for seconds.
#[derive(Debug, Default)]
struct Shapes {
    /// Keys of the valid entries, kept apart from them for a fast search.
    keys: Vec<ShapeKey>,
    /// Built shapes; the first `keys.len()` are valid, the others keep
    /// their buffers for reuse.
    entries: Vec<Shape>,
    /// Entry of every box of the current call.
    slots: Vec<usize>,
    /// Gaussian kernels by sigma.
    kernels: Kernels,
}

/// The shape key of a box (see [`PreparedBox::shape_key`]) followed by
/// the fill's sigma.
type ShapeKey = [i64; 16];

/// The mask terms and block grids of one shape at luma (index 0) and
/// chroma (index 1) resolution.
#[derive(Debug, Default)]
struct Shape {
    masks: [ChannelMask; 2],
    geometries: [Geometry; 2],
}

/// Intermediate values of one target plane.
#[derive(Debug, Default)]
struct Work {
    /// Column sums of the current band of rows, one row per channel.
    sums: Vec<f32>,
    /// Block means, per channel one row per vertical cell.
    means: Vec<f32>,
    /// The rows every box is filled from, box after box (see
    /// [`Job::fill_rows`]).
    rows: Vec<f32>,
    /// Start of each box's rows in `rows`.
    starts: Vec<usize>,
    /// One upsampled row of means with its replicated border.
    padded_row: Vec<f32>,
    /// Mask of one row in a corner band.
    row_mask: Vec<f32>,
}

/// Blurs validated `boxes` into `frame` with the given buffers.
fn blur_boxes(
    frame: &mut YuvFrameMut<'_>,
    origin: (i32, i32),
    boxes: &[PreparedBox],
    params: &PixelParams,
    scratch: &mut Scratch,
) {
    let Scratch { shapes, work } = scratch;
    shapes.build(boxes, params);
    for target in frame.targets() {
        match target.layout {
            LayoutKind::Packed8 => {
                blur_target::<Packed8, 1>(frame, &target, origin, boxes, params, shapes, work);
            }
            LayoutKind::Packed10 => {
                blur_target::<Packed10, 1>(frame, &target, origin, boxes, params, shapes, work);
            }
            LayoutKind::Pair8 => {
                blur_target::<Pair8, 2>(frame, &target, origin, boxes, params, shapes, work);
            }
            LayoutKind::Pair10 => {
                blur_target::<Pair10, 2>(frame, &target, origin, boxes, params, shapes, work);
            }
        }
    }
}

impl Shapes {
    /// Shapes kept across calls; beyond that the store starts over at the
    /// next call. One call always gets all of its shapes.
    const CAPACITY: usize = 64;

    /// Finds or builds the shape of every box of a call.
    fn build(&mut self, boxes: &[PreparedBox], params: &PixelParams) {
        if self.keys.len() > Self::CAPACITY {
            self.keys.clear();
        }
        // The fill decides whether the grids carry interpolation weights
        // and with which sigma.
        let fill = match params.fill {
            Fill::Smooth { sigma_per_block } => i64::from(sigma_per_block.to_bits()),
            Fill::Blocks | Fill::BoxMean | Fill::Colour(..) => -1,
        };
        self.slots.clear();
        for b in boxes {
            let mut key = [fill; 16];
            key[..15].copy_from_slice(&b.shape_key());
            let found = self.keys.iter().position(|k| *k == key);
            let slot = found.unwrap_or_else(|| {
                let slot = self.keys.len();
                if self.entries.len() == slot {
                    self.entries.push(Shape::default());
                }
                self.entries[slot].build(b, params.fill, &mut self.kernels);
                self.keys.push(key);
                slot
            });
            self.slots.push(slot);
        }
    }

    /// The shape of box `i` of the current call.
    fn of(&self, i: usize) -> &Shape {
        &self.entries[self.slots[i]]
    }
}

impl Shape {
    /// Computes the masks and grids of box `b`.
    fn build(&mut self, b: &PreparedBox, fill: Fill, kernels: &mut Kernels) {
        let [luma, chroma] = &mut self.masks;
        luma.build_luma(b);
        chroma.build_chroma(luma);
        for (geometry, subsampling) in self
            .geometries
            .iter_mut()
            .zip([Subsampling::Luma, Subsampling::Chroma])
        {
            let kernel = match fill {
                Fill::Smooth { sigma_per_block } => {
                    kernels.get(sigma_per_block * b.block / subsampling.factor() as f32)
                }
                Fill::Blocks | Fill::BoxMean | Fill::Colour(..) => &[],
            };
            geometry.build(b, subsampling, kernel);
        }
    }
}

/// Blurs every box into one target plane whose pixels have `N` channels
/// stored as `L`.
fn blur_target<L: Layout<N>, const N: usize>(
    frame: &mut YuvFrameMut<'_>,
    target: &Target,
    origin: (i32, i32),
    boxes: &[PreparedBox],
    params: &PixelParams,
    shapes: &Shapes,
    work: &mut Work,
) {
    let subsampling = target.subsampling();
    let s = subsampling.factor();
    let plane = PlaneAccess {
        stride: target.stride,
        offset: (origin.0 / s, origin.1 / s),
    };
    let resolution = usize::from(subsampling == Subsampling::Chroma);

    // Everything is read from the source before anything is written.
    let source: &[u8] = frame.plane_data(target.plane);
    work.rows.clear();
    work.starts.clear();
    for (i, b) in boxes.iter().enumerate() {
        work.starts.push(work.rows.len());
        let job = Job {
            source,
            plane,
            target,
            area: Area::of(b, subsampling),
            params,
            geometry: &shapes.of(i).geometries[resolution],
            b,
        };
        job.fill_rows::<L, N>(work);
    }

    let data = frame.plane_data(target.plane);
    let Work {
        rows,
        starts,
        row_mask,
        ..
    } = work;
    for (i, (b, &start)) in boxes.iter().zip(starts.iter()).enumerate() {
        let shape = shapes.of(i);
        let area = Area::of(b, subsampling);
        let fill = Filled {
            fill: params.fill,
            geometry: &shape.geometries[resolution],
            rows: &rows[start..],
            width: area.width(),
        };
        let mixer = Mixer {
            data: &mut *data,
            plane,
            area,
            b,
            mask: &shape.masks[resolution],
            max: target.max_value(),
        };
        mixer.composite::<L, N>(&fill, row_mask);
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

/// Where the pixels of a target plane are, relative to full-frame channel
/// coordinates.
#[derive(Debug, Clone, Copy)]
struct PlaneAccess {
    /// Bytes from one row to the next.
    stride: usize,
    /// Full-frame channel coordinates of the view's first pixel.
    offset: (i32, i32),
}

impl PlaneAccess {
    /// Byte index of pixel `(x, y)` (full-frame channel coordinates) in a
    /// plane with `step` bytes per pixel.
    const fn index(&self, x: i32, y: i32, step: usize) -> usize {
        (y - self.offset.1) as usize * self.stride + (x - self.offset.0) as usize * step
    }
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

/// The source side of one box in one target plane.
struct Job<'a> {
    source: &'a [u8],
    plane: PlaneAccess,
    target: &'a Target,
    area: Area,
    params: &'a PixelParams,
    /// Grid cells and interpolation weights of the box at this resolution.
    geometry: &'a Geometry,
    b: &'a PreparedBox,
}

impl Job<'_> {
    /// Sums the source pixels of full-frame channel rows `y0..y1` per column
    /// of the area into `sums` (one row of the area's width per channel).
    fn sum_band<L: Layout<N>, const N: usize>(&self, (y0, y1): (i32, i32), sums: &mut [f32]) {
        let band = Band {
            data: self.source,
            first: self.plane.index(self.area.x0, y0, L::STEP),
            stride: self.plane.stride,
            rows: (y1 - y0) as usize,
        };
        band.sum::<L, N>(self.area.width(), sums);
    }

    /// Appends the rows the box is filled from to `work.rows`:
    ///
    /// - smooth fill: per channel the horizontally upsampled and blurred
    ///   row of every cell row; output rows are weighted sums of them;
    /// - blocks: per channel the expanded means of every cell row;
    /// - box mean and solid colour: per channel one constant row.
    ///
    /// Noise can push values slightly out of range; the mix clamps.
    fn fill_rows<L: Layout<N>, const N: usize>(&self, work: &mut Work) {
        let w = self.area.width();
        match self.params.fill {
            Fill::Smooth { .. } => {
                self.block_means::<L, N>(work);
                smooth(work, self.geometry, w, N);
            }
            Fill::Blocks => {
                self.block_means::<L, N>(work);
                let cols = &self.geometry.cols;
                for means in work.means.chunks_exact(cols.len()) {
                    for (c, &mean) in means.iter().enumerate() {
                        let (x0, x1) = (cols.bounds[c], cols.bounds[c + 1]);
                        let len = (x1 - x0) as usize;
                        work.rows.extend(std::iter::repeat_n(mean, len));
                    }
                }
            }
            Fill::BoxMean => {
                work.sums.resize(N * w, 0.0);
                self.sum_band::<L, N>((self.area.y0, self.area.y1), &mut work.sums);
                let len = self.area.len() as f32;
                for sums in work.sums.chunks_exact(w) {
                    let mean = lane_sum(sums) / len;
                    work.rows.extend(std::iter::repeat_n(mean, w));
                }
            }
            Fill::Colour(colour, rgb) => {
                let bits = if self.target.max_value() > 255.0 {
                    10
                } else {
                    8
                };
                let values = colour.code_values(rgb, bits);
                for &channel in self.target.channels {
                    work.rows.extend(std::iter::repeat_n(values[channel], w));
                }
            }
        }
    }

    /// Computes the block means of the area into `work.means` (per channel
    /// one row per vertical cell). Adds the per-block noise to luma when it
    /// is enabled.
    fn block_means<L: Layout<N>, const N: usize>(&self, work: &mut Work) {
        let w = self.area.width();
        let (cols, rows) = (&self.geometry.cols, &self.geometry.rows);
        let (nx, ny) = (cols.len(), rows.len());
        work.means.clear();
        work.means.resize(N * ny * nx, 0.0);
        work.sums.resize(N * w, 0.0);
        let amplitude = NOISE_AMPLITUDE * self.target.max_value() / 255.0;
        for r in 0..ny {
            let (y0, y1) = (rows.bounds[r], rows.bounds[r + 1]);
            let band = (self.area.y0 + y0, self.area.y0 + y1);
            self.sum_band::<L, N>(band, &mut work.sums);
            let height = (y1 - y0) as f32;
            for (c, sums) in work.sums.chunks_exact(w).enumerate() {
                let noisy = self.params.noise && self.target.channels[c] == 0;
                let means = &mut work.means[(c * ny + r) * nx..][..nx];
                for (col, mean) in means.iter_mut().enumerate() {
                    let x0 = cols.bounds[col] as usize;
                    let x1 = cols.bounds[col + 1] as usize;
                    let sum: f32 = sums[x0..x1].iter().sum();
                    *mean = sum / (height * (x1 - x0) as f32);
                    if noisy {
                        *mean += amplitude * noise(self.b.seed, cols.index(col), rows.index(r));
                    }
                }
            }
        }
    }
}

/// The rows a box is filled from (see [`Job::fill_rows`]) and how output
/// rows are made of them.
struct Filled<'a> {
    fill: Fill,
    geometry: &'a Geometry,
    /// The box's rows, from the start of them.
    rows: &'a [f32],
    width: usize,
}

impl Filled<'_> {
    /// The cell rows of channel `c`, `first..=last`.
    fn cell_rows(&self, c: usize, (first, last): (usize, usize)) -> &[f32] {
        let (w, ny) = (self.width, self.geometry.rows.len());
        &self.rows[(c * ny + first) * w..(c * ny + last + 1) * w]
    }

    /// The smooth fill of output row `y` (area row index): a weighted sum
    /// of the few cell rows that reach it.
    fn weighted<const N: usize>(&self, y: usize) -> Weighted<'_, N> {
        let geometry = self.geometry;
        let band = geometry.bands[y];
        Weighted {
            weights: &geometry.vertical[y * geometry.rows.len()..][band.0..=band.1],
            rows: std::array::from_fn(|c| self.cell_rows(c, band)),
            width: self.width,
        }
    }

    /// The hard blocks of output row `y` (area row index).
    fn blocks<const N: usize>(&self, y: usize) -> Direct<'_, N> {
        let r = self.geometry.cell_rows[y];
        Direct(std::array::from_fn(|c| self.cell_rows(c, (r, r))))
    }

    /// The constant row of a box-mean or colour fill.
    fn constant<const N: usize>(&self) -> Direct<'_, N> {
        let w = self.width;
        Direct(std::array::from_fn(|c| &self.rows[c * w..(c + 1) * w]))
    }
}

/// The blurred values of one output row, per channel.
trait Blurred<const N: usize> {
    /// The values of pixels `x..x + 8`.
    fn load8(&self, x: usize) -> [f32x8; N];
    /// The values of pixel `x`.
    fn get(&self, x: usize) -> [f32; N];
}

/// A row stored as it is.
struct Direct<'a, const N: usize>([&'a [f32]; N]);

impl<const N: usize> Blurred<N> for Direct<'_, N> {
    fn load8(&self, x: usize) -> [f32x8; N] {
        self.0.map(|row| load8(&row[x..]))
    }

    fn get(&self, x: usize) -> [f32; N] {
        self.0.map(|row| row[x])
    }
}

/// `sum_r weights[r] * rows[c][r * width + x]`, computed while mixing, so
/// the output row is never stored.
struct Weighted<'a, const N: usize> {
    weights: &'a [f32],
    /// Per channel the cell rows that reach the output row.
    rows: [&'a [f32]; N],
    width: usize,
}

impl<const N: usize> Blurred<N> for Weighted<'_, N> {
    fn load8(&self, x: usize) -> [f32x8; N] {
        self.rows.map(|rows| {
            let mut acc = f32x8::ZERO;
            for (r, &w) in self.weights.iter().enumerate() {
                acc += f32x8::splat(w) * load8(&rows[r * self.width + x..]);
            }
            acc
        })
    }

    fn get(&self, x: usize) -> [f32; N] {
        self.rows.map(|rows| {
            let mut acc = 0.0;
            for (r, &w) in self.weights.iter().enumerate() {
                acc += w * rows[r * self.width + x];
            }
            acc
        })
    }
}

/// A box's block grid and interpolation weights at one resolution, in
/// coordinates relative to the box's padded area.
#[derive(Debug, Default)]
struct Geometry {
    /// Grid cells along x and y.
    cols: Cells,
    rows: Cells,
    /// Cell row of every area row.
    cell_rows: Vec<usize>,
    /// Horizontal bilinear taps per output column: the two cells and the
    /// weight of the second.
    tap_first: Vec<u32>,
    tap_second: Vec<u32>,
    tap_weight: Vec<f32>,
    /// Gaussian weights for the horizontal pass.
    kernel: Vec<f32>,
    /// Vertical bilinear taps per area row, like the horizontal ones.
    row_taps: Vec<(usize, usize, f32)>,
    /// Folded vertical weights, one per cell row for every output row.
    vertical: Vec<f32>,
    /// Per output row, the first and last cell row with a non-zero weight.
    bands: Vec<(usize, usize)>,
}

impl Geometry {
    /// Computes the grid of box `b` at the given resolution and, when
    /// `kernel` is not empty (smooth fill), the interpolation weights.
    fn build(&mut self, b: &PreparedBox, subsampling: Subsampling, kernel: &[f32]) {
        let area = Area::of(b, subsampling);
        let s = subsampling.factor();
        let block = b.block / s as f32;
        // The grid origin relative to the area. The padded box starts on
        // even coordinates, so this is exact for chroma as well.
        let local = |g: i32, start: i32| (i64::from(g) - i64::from(s * start)) as f32 / s as f32;
        let (gx, gy) = b.grid_origin;
        let (width, height) = (area.width() as i32, area.height() as i32);
        self.cols.compute(local(gx, area.x0), block, 0, width);
        self.rows.compute(local(gy, area.y0), block, 0, height);
        self.cell_rows.clear();
        for (r, bounds) in self.rows.bounds.windows(2).enumerate() {
            let len = (bounds[1] - bounds[0]) as usize;
            self.cell_rows.extend(std::iter::repeat_n(r, len));
        }
        self.kernel.clear();
        self.kernel.extend_from_slice(kernel);
        if kernel.is_empty() {
            return;
        }
        self.tap_first.clear();
        self.tap_second.clear();
        self.tap_weight.clear();
        bilinear_taps(&self.cols, area.width(), |c0, c1, t| {
            self.tap_first.push(c0 as u32);
            self.tap_second.push(c1 as u32);
            self.tap_weight.push(t);
        });

        // A = G * U: the vertical bilinear weights folded into the gaussian
        // (replicated border), one row of cell weights per output row.
        let (height, ny) = (area.height(), self.rows.len());
        let radius = kernel.len() / 2;
        self.row_taps.clear();
        bilinear_taps(&self.rows, height, |r0, r1, t| {
            self.row_taps.push((r0, r1, t));
        });
        self.vertical.clear();
        self.vertical.resize(height * ny, 0.0);
        self.bands.clear();
        for (y, weights) in self.vertical.chunks_exact_mut(ny).enumerate() {
            // Consecutive taps mostly share their pair of cells: sum them in
            // registers and write when the pair changes.
            let (mut pair, mut sum0, mut sum1) = ((0, 0), 0.0f32, 0.0f32);
            for (t, &k) in kernel.iter().enumerate() {
                let source = (y + t).saturating_sub(radius).min(height - 1);
                let (r0, r1, w) = self.row_taps[source];
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

/// Bilinear upsample of the block means followed by a horizontal gaussian
/// with a replicated border: appends the smoothed row of every cell row of
/// every channel (`channels` sets of means in `work.means`) to `work.rows`.
///
/// Each row of means is upsampled to the area's `width`, padded by the
/// kernel radius with its edge values and convolved. Output row `y` is then
/// `sum_k A[y][k] * row_k` with the folded weights of [`Geometry`], which
/// equals upsampling the whole area first, because both passes are linear
/// and act on different axes.
fn smooth(work: &mut Work, geometry: &Geometry, width: usize, channels: usize) {
    let nx = geometry.cols.len();
    let kernel = &geometry.kernel;
    let radius = kernel.len() / 2;
    let start = work.rows.len();
    let rows = work.means.len() / nx;
    debug_assert_eq!(rows, channels * geometry.rows.len());
    work.rows.resize(start + rows * width, 0.0);
    work.padded_row.resize(width + 2 * radius, 0.0);
    let padded = &mut work.padded_row;
    for (means, out) in work
        .means
        .chunks_exact(nx)
        .zip(work.rows[start..].chunks_exact_mut(width))
    {
        upsample(means, geometry, &mut padded[radius..radius + width]);
        let (left, right) = (padded[radius], padded[radius + width - 1]);
        padded[..radius].fill(left);
        padded[radius + width..].fill(right);
        convolve(padded, kernel, out);
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

/// The first eight values of `values` as a vector.
fn load8(values: &[f32]) -> f32x8 {
    let mut lanes = [0.0; 8];
    lanes.copy_from_slice(&values[..8]);
    f32x8::from(lanes)
}

/// Consecutive rows of a plane.
struct Band<'a> {
    data: &'a [u8],
    /// Byte index of the first pixel of the first row.
    first: usize,
    /// Bytes from one row to the next.
    stride: usize,
    rows: usize,
}

impl Band<'_> {
    /// Sums the first `width` pixels over the rows, per column and channel:
    /// `sums[c * width + i]` is the sum of channel `c` of pixel `i`. Up to
    /// four groups of eight pixels are summed at once (fewer for pairs, to
    /// stay within the vector registers), so the additions of one group do
    /// not wait for each other.
    fn sum<L: Layout<N>, const N: usize>(&self, width: usize, sums: &mut [f32]) {
        let groups = 4 / N;
        let mut column = 0;
        while column + 8 * groups <= width {
            let mut acc = [[f32x8::ZERO; N]; 4];
            for row in 0..self.rows {
                let start = self.first + row * self.stride + column * L::STEP;
                for (g, a) in acc.iter_mut().take(groups).enumerate() {
                    let values = L::load8(&self.data[start + 8 * g * L::STEP..]);
                    for (a, v) in a.iter_mut().zip(values) {
                        *a += v;
                    }
                }
            }
            for (g, a) in acc.iter().take(groups).enumerate() {
                for (c, a) in a.iter().enumerate() {
                    let at = c * width + column + 8 * g;
                    sums[at..at + 8].copy_from_slice(&a.to_array());
                }
            }
            column += 8 * groups;
        }
        while column + 8 <= width {
            let mut acc = [f32x8::ZERO; N];
            for row in 0..self.rows {
                let values =
                    L::load8(&self.data[self.first + row * self.stride + column * L::STEP..]);
                for (a, v) in acc.iter_mut().zip(values) {
                    *a += v;
                }
            }
            for (c, a) in acc.iter().enumerate() {
                let at = c * width + column;
                sums[at..at + 8].copy_from_slice(&a.to_array());
            }
            column += 8;
        }
        for i in column..width {
            let mut acc = [0.0f32; N];
            for row in 0..self.rows {
                let values = L::load(&self.data[self.first + row * self.stride + i * L::STEP..]);
                for (a, v) in acc.iter_mut().zip(values) {
                    *a += v;
                }
            }
            for (c, a) in acc.into_iter().enumerate() {
                sums[c * width + i] = a;
            }
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

/// Calls `tap` for each of the first `len` samples with the two cells to
/// interpolate and the weight of the second one. Samples before the first
/// or after the last cell centre take that cell's value (replicated
/// border).
fn bilinear_taps(cells: &Cells, len: usize, mut tap: impl FnMut(usize, usize, f32)) {
    let c = &cells.centres;
    let last = c.len() - 1;
    let mut j = 0;
    for i in 0..len {
        let p = i as f32 + 0.5;
        while j < last && c[j + 1] <= p {
            j += 1;
        }
        if p <= c[0] {
            tap(0, 0, 0.0);
        } else if j == last {
            tap(last, last, 0.0);
        } else {
            tap(j, j + 1, (p - c[j]) / (c[j + 1] - c[j]));
        }
    }
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
    /// Computes the luma terms of box `b`.
    fn build_luma(&mut self, b: &PreparedBox) {
        let padded = b.padded();
        self.chroma = false;
        self.columns.clear();
        self.columns.extend((padded.x0..padded.x1).map(|x| {
            let term = b.column(x);
            [term, term]
        }));
        self.rows.clear();
        self.rows.extend((padded.y0..padded.y1).map(|y| {
            let term = b.row(y);
            [term, term]
        }));
        self.pool();
    }

    /// Computes the chroma terms from the luma terms of the same box:
    /// chroma sample `i` covers luma columns (rows) `2i` and `2i + 1` of the
    /// padded box, which starts on an even coordinate; a second column (row)
    /// past an odd frame edge is outside the box.
    fn build_chroma(&mut self, luma: &Self) {
        self.chroma = true;
        self.columns.clear();
        self.columns.extend(luma_pairs(&luma.columns));
        self.rows.clear();
        self.rows.extend(luma_pairs(&luma.rows));
        self.pool();
    }

    /// Derives the max-pooled column masks and the corner columns.
    fn pool(&mut self) {
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

/// The chroma terms of luma terms: pairs of consecutive luma columns
/// (rows), the second one outside the box past an odd frame edge.
fn luma_pairs(terms: &[[AxisMask; 2]]) -> impl Iterator<Item = [AxisMask; 2]> + '_ {
    terms.chunks(2).map(|pair| match pair {
        [[first, _], [second, _]] => [*first, *second],
        [[first, _]] => [*first, AxisMask::OUTSIDE],
        _ => [AxisMask::OUTSIDE; 2],
    })
}

/// Mixes a box's blurred rows into a target plane through its mask.
struct Mixer<'a> {
    data: &'a mut [u8],
    plane: PlaneAccess,
    area: Area,
    b: &'a PreparedBox,
    mask: &'a ChannelMask,
    /// Largest sample value.
    max: f32,
}

impl Mixer<'_> {
    /// Mixes every row of the box with the blurred values of `fill`.
    fn composite<L: Layout<N>, const N: usize>(self, fill: &Filled<'_>, row_mask: &mut Vec<f32>) {
        match fill.fill {
            Fill::Smooth { .. } => self.mix_rows::<L, N, _>(|y| fill.weighted(y), row_mask),
            Fill::Blocks => self.mix_rows::<L, N, _>(|y| fill.blocks(y), row_mask),
            Fill::BoxMean | Fill::Colour(..) => {
                self.mix_rows::<L, N, _>(|_| fill.constant(), row_mask);
            }
        }
    }

    /// Mixes every row of the box, with `blurred(y)` giving the values of
    /// area row `y`.
    ///
    /// The mask is the minimum of a column and a row term everywhere except
    /// in the rounded corners (see [`PreparedBox::pixel_mask`]), so rows
    /// outside the corner bands use one row value against the precomputed
    /// column profile. Chroma takes the maximum over the 2×2 luma pixels of
    /// each sample; outside the corners that is the minimum of the
    /// max-pooled column and row terms.
    fn mix_rows<L: Layout<N>, const N: usize, B: Blurred<N>>(
        self,
        blurred: impl Fn(usize) -> B,
        row_mask: &mut Vec<f32>,
    ) {
        let mask = self.mask;
        for (i, (y, &[r0, r1])) in (self.area.y0..self.area.y1).zip(&mask.rows).enumerate() {
            let start = self.plane.index(self.area.x0, y, L::STEP);
            let pooled_row = r0.mask.max(r1.mask);
            if r0.corner || r1.corner {
                // Full mask for this row: separable terms, then the corners.
                row_mask.clear();
                row_mask.extend(mask.pooled.iter().map(|&m| m.min(pooled_row)));
                for &c in &mask.corners {
                    let [c0, c1] = mask.columns[c];
                    row_mask[c] = if mask.chroma {
                        self.b
                            .pixel_mask(c0, r0)
                            .max(self.b.pixel_mask(c1, r0))
                            .max(self.b.pixel_mask(c0, r1))
                            .max(self.b.pixel_mask(c1, r1))
                    } else {
                        self.b.pixel_mask(c0, r0)
                    };
                }
                mix_row::<L, N>(self.data, start, &blurred(i), row_mask, 1.0, self.max);
            } else {
                let mask = &mask.pooled;
                mix_row::<L, N>(self.data, start, &blurred(i), mask, pooled_row, self.max);
            }
        }
    }
}

/// Mixes one row in place: channel `c` of pixel `i` (from byte `start`)
/// becomes `round(clamp(src + (blurred[c][i] - src) * min(mask[i],
/// row_mask)))`, eight pixels at a time.
fn mix_row<L: Layout<N>, const N: usize>(
    data: &mut [u8],
    start: usize,
    blurred: &impl Blurred<N>,
    mask: &[f32],
    row_mask: f32,
    max: f32,
) {
    let width = mask.len();
    let (zero, half) = (f32x8::ZERO, f32x8::splat(0.5));
    let (row_v, max_v) = (f32x8::splat(row_mask), f32x8::splat(max));
    let (mask_chunks, mask_rest) = mask.as_chunks::<8>();
    for (n, m) in mask_chunks.iter().enumerate() {
        let chunk = &mut data[start + n * 8 * L::STEP..][..8 * L::STEP];
        let source = L::load8(chunk);
        let values = blurred.load8(n * 8);
        let m = f32x8::from(*m).fast_min(row_v);
        let mixed = std::array::from_fn(|c| {
            let (b, s) = (values[c], source[c]);
            // All values are finite, so the fast (not NaN-aware) operations
            // are exact here.
            (((b - s) * m + s).fast_max(zero).fast_min(max_v) + half).fast_trunc_int()
        });
        L::store8(chunk, mixed);
    }
    let done = width - mask_rest.len();
    for (k, &m) in mask_rest.iter().enumerate() {
        let i = done + k;
        let pixel = &mut data[start + i * L::STEP..];
        let (source, values) = (L::load(pixel), blurred.get(i));
        let m = m.min(row_mask);
        let mixed = std::array::from_fn(|c| {
            let (b, s) = (values[c], source[c]);
            (((b - s) * m + s).clamp(0.0, max) + 0.5) as i32
        });
        L::store(pixel, mixed);
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
    fn stored_shapes_match_fresh_ones() {
        let rect = RectI::new(20, 10, 60, 24).unwrap();
        let params = BlurParams::default();
        // A new thread starts with an empty shape store.
        let fresh = std::thread::spawn(move || run(&params, rect, 96, 40))
            .join()
            .unwrap();
        // Fill this thread's store beyond its capacity with other shapes,
        // then blur the box once with a newly built and once with a stored
        // shape.
        for width in 0..70 {
            run(&params, RectI::new(4, 4, 10 + width, 20).unwrap(), 96, 40);
        }
        assert_eq!(run(&params, rect, 96, 40), fresh);
        assert_eq!(run(&params, rect, 96, 40), fresh);
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
