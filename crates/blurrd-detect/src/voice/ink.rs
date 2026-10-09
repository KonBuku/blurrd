//! Text ink: how far each pixel stands out from its blurred surroundings.
//!
//! The prototype measures text as `max_c |I_c − gauss_σ2.5(I_c)|` (port
//! spec section 5): thin glyph strokes and portrait edges differ strongly
//! from a local mean, smooth game backgrounds do not. Taking the maximum
//! over the three channels makes the measure independent of the hue of
//! text and background, so colour-blind modes and colour filters change it
//! little; brightness filters scale it with the contrast.
//!
//! The Gaussian is approximated by three passes of a 5-tap box filter in
//! each direction (variance 3 × 2 = 6, σ ≈ 2.45), computed with
//! integer sums: exact, deterministic and cheap. The passes run on the
//! measured area plus a margin of their combined reach, read with the
//! crop's edge pixels repeated, and each pass keeps only the samples whose
//! window lies fully inside its input, so no pass needs bounds handling.

use std::ops::AddAssign;

use blurrd_core::RectI;

use crate::image::RgbView;

/// Radius of one box pass.
const BOX_RADIUS: usize = 2;
/// Taps of one box pass.
const BOX_TAPS: u32 = 2 * BOX_RADIUS as u32 + 1;
/// Number of box passes per direction.
const PASSES: usize = 3;
/// Pixels read beyond the measured area on each side: the reach of all
/// passes, so that the area itself never sees a clamped border unless the
/// crop ends there.
pub(crate) const MARGIN: i32 = (PASSES * BOX_RADIUS) as i32;
/// Scale of the blurred sums: `BOX_TAPS^(2 · PASSES)`.
const SCALE: u32 = BOX_TAPS.pow(2 * PASSES as u32);

/// The ink and darkest-channel planes of an area of a crop.
#[derive(Debug, Clone)]
pub(crate) struct InkPlanes {
    /// The measured area in crop coordinates.
    area: RectI,
    /// `max_c |I_c − blur(I_c)|` per pixel of the area, row after row.
    ink: Vec<u8>,
    /// `min(R, G, B)` per pixel of the area.
    lmin: Vec<u8>,
    /// Luma (BT.601 weights) per pixel of the area.
    luma: Vec<u8>,
    /// Scratch: the byte offset in a crop row of every padded column.
    offsets: Vec<usize>,
    /// Scratch: one channel of the padded area and the other buffer of
    /// the horizontal passes (their sums fit 16 bits: 255 · 5³).
    rows: [Vec<u16>; 2],
    /// Scratch: the two buffers of the vertical passes.
    columns: [Vec<u32>; 2],
    /// Scratch: the largest scaled difference over the channels.
    largest: Vec<u32>,
}

impl Default for InkPlanes {
    fn default() -> Self {
        Self {
            area: RectI {
                x0: 0,
                y0: 0,
                x1: 0,
                y1: 0,
            },
            ink: Vec::new(),
            lmin: Vec::new(),
            luma: Vec::new(),
            offsets: Vec::new(),
            rows: [Vec::new(), Vec::new()],
            columns: [Vec::new(), Vec::new()],
            largest: Vec::new(),
        }
    }
}

impl InkPlanes {
    /// Measures `area` (crop coordinates, inside the crop) of `crop`.
    pub(crate) fn compute(&mut self, crop: &RgbView<'_>, area: RectI) {
        self.area = area;
        let (width, height) = (area.width() as usize, area.height() as usize);
        // The padded area, clamped to the crop; reads beyond the crop
        // repeat its edge pixels.
        let pad = RectI {
            x0: area.x0 - MARGIN,
            y0: area.y0 - MARGIN,
            x1: area.x1 + MARGIN,
            y1: area.y1 + MARGIN,
        };
        let (pad_w, pad_h) = (pad.width() as usize, pad.height() as usize);
        let max_x = crop.width() as i32 - 1;
        let max_y = crop.height() as i32 - 1;

        self.ink.clear();
        self.ink.resize(width * height, 0);
        self.lmin.clear();
        self.lmin.resize(width * height, 0);
        self.luma.clear();
        self.luma.resize(width * height, 0);
        for (y, (lmin_row, luma_row)) in self
            .lmin
            .chunks_exact_mut(width)
            .zip(self.luma.chunks_exact_mut(width))
            .enumerate()
        {
            let row = crop.row((area.y0 + y as i32) as u32);
            let pixels = &row[3 * area.x0 as usize..3 * area.x1 as usize];
            for ((lmin, luma), px) in lmin_row
                .iter_mut()
                .zip(luma_row.iter_mut())
                .zip(pixels.as_chunks::<3>().0)
            {
                let [r, g, b] = px.map(u32::from);
                *lmin = r.min(g).min(b) as u8;
                *luma = ((299 * r + 587 * g + 114 * b + 500) / 1000) as u8;
            }
        }

        self.offsets.clear();
        self.offsets
            .extend((pad.x0..pad.x1).map(|x| 3 * x.clamp(0, max_x) as usize));
        self.largest.clear();
        self.largest.resize(width * height, 0);
        for c in 0..3 {
            let [rows, rows_swap] = &mut self.rows;
            rows.clear();
            rows.resize(pad_w * pad_h, 0);
            for (py, out) in rows.chunks_exact_mut(pad_w).enumerate() {
                let y = (pad.y0 + py as i32).clamp(0, max_y);
                let row = &crop.row(y as u32)[c..];
                for (v, &offset) in out.iter_mut().zip(&self.offsets) {
                    *v = u16::from(row[offset]);
                }
            }
            let mut w = pad_w;
            for _ in 0..PASSES {
                box_rows(rows, rows_swap, w);
                std::mem::swap(rows, rows_swap);
                w -= 2 * BOX_RADIUS;
            }
            let [columns, columns_swap] = &mut self.columns;
            columns.clear();
            columns.extend(rows.iter().map(|&v| u32::from(v)));
            let mut h = pad_h;
            for _ in 0..PASSES {
                box_columns(columns, columns_swap, w, h);
                std::mem::swap(columns, columns_swap);
                h -= 2 * BOX_RADIUS;
            }
            for (y, largest) in self.largest.chunks_exact_mut(width).enumerate() {
                let row = crop.row((area.y0 + y as i32) as u32);
                let pixels = &row[3 * area.x0 as usize + c..];
                let blurred = &columns[y * width..(y + 1) * width];
                for ((largest, &sum), x) in largest.iter_mut().zip(blurred).zip(0..) {
                    let v = u32::from(pixels[3 * x]) * SCALE;
                    *largest = (*largest).max(v.abs_diff(sum));
                }
            }
        }
        // Rounding is monotonic: the rounded maximum is the maximum of the
        // rounded channel differences.
        for (ink, &d) in self.ink.iter_mut().zip(&self.largest) {
            *ink = ((d + SCALE / 2) / SCALE).min(255) as u8;
        }
    }

    /// Ink at area pixel `(x, y)` given in crop coordinates.
    fn index(&self, x: i32, y: i32) -> usize {
        (y - self.area.y0) as usize * self.area.width() as usize + (x - self.area.x0) as usize
    }

    /// The part of `rect` (crop coordinates) inside the measured area.
    fn clip(&self, rect: RectI) -> Option<RectI> {
        rect.intersection(&self.area)
    }

    /// Number of pixels of `rect` (crop coordinates) with ink above
    /// `threshold`, and the number of pixels of `rect` inside the area.
    pub(crate) fn count_above(&self, rect: RectI, threshold: u8) -> (u32, u32) {
        let Some(r) = self.clip(rect) else {
            return (0, 0);
        };
        let mut n = 0;
        for y in r.y0..r.y1 {
            let start = self.index(r.x0, y);
            let row = &self.ink[start..start + r.width() as usize];
            n += row.iter().filter(|&&v| v > threshold).count() as u32;
        }
        (n, r.width() * r.height())
    }

    /// Per column of `rect` (crop coordinates), the number of pixels with
    /// ink above `threshold`; `out[i]` belongs to column `rect.x0 + i`.
    pub(crate) fn column_counts(&self, rect: RectI, threshold: u8, out: &mut Vec<u8>) {
        out.clear();
        out.resize(rect.width() as usize, 0);
        let Some(r) = self.clip(rect) else {
            return;
        };
        for y in r.y0..r.y1 {
            let start = self.index(r.x0, y);
            let row = &self.ink[start..start + r.width() as usize];
            let offset = (r.x0 - rect.x0) as usize;
            for (count, &v) in out[offset..].iter_mut().zip(row) {
                *count = count.saturating_add(u8::from(v > threshold));
            }
        }
    }

    /// The `p`-quantile (`0.0..=1.0`) and the median of the darkest
    /// channel over `rect` (crop coordinates), or `None` if `rect` misses
    /// the area.
    pub(crate) fn lmin_quantile_and_median(&self, rect: RectI, p: f32) -> Option<(u8, u8)> {
        let r = self.clip(rect)?;
        let mut histogram = [0u32; 256];
        for y in r.y0..r.y1 {
            let start = self.index(r.x0, y);
            for &v in &self.lmin[start..start + r.width() as usize] {
                histogram[usize::from(v)] += 1;
            }
        }
        let n = r.width() * r.height();
        // The value at sorted index `k`, as `np.partition` picks it.
        let at = |k: u32| -> u8 {
            let mut seen = 0;
            for (v, &count) in histogram.iter().enumerate() {
                seen += count;
                if seen > k {
                    return v as u8;
                }
            }
            u8::MAX
        };
        let k = ((n as f32 * p) as u32).min(n - 1);
        Some((at(k), at(n / 2)))
    }

    /// Marks in `out` the pixels of row `y` from `x0` to `x1` (crop
    /// coordinates, clipped to the area) with ink above `ink` and a darkest
    /// channel of at least `lmin`: bright strokes. `out[i]` belongs to
    /// column `x0 + i`.
    pub(crate) fn bright_strokes(
        &self,
        y: i32,
        x0: i32,
        x1: i32,
        ink: u8,
        lmin: u8,
        out: &mut Vec<bool>,
    ) {
        out.clear();
        out.resize(usize::try_from(x1 - x0).unwrap_or(0), false);
        if y < self.area.y0 || y >= self.area.y1 {
            return;
        }
        let (from, to) = (x0.max(self.area.x0), x1.min(self.area.x1));
        for x in from..to {
            let i = self.index(x, y);
            out[(x - x0) as usize] = self.ink[i] > ink && self.lmin[i] >= lmin;
        }
    }

    /// Ink at pixel `(x, y)` (crop coordinates, inside the area).
    #[cfg(test)]
    fn ink_at(&self, x: i32, y: i32) -> u8 {
        self.ink[self.index(x, y)]
    }

    /// Luma at pixel `(x, y)` (crop coordinates); 0 outside the area.
    pub(crate) fn luma(&self, x: i32, y: i32) -> u8 {
        if x < self.area.x0 || x >= self.area.x1 || y < self.area.y0 || y >= self.area.y1 {
            return 0;
        }
        self.luma[self.index(x, y)]
    }
}

/// One horizontal box pass over rows of `width` samples (at least
/// [`BOX_TAPS`]): `dst[x] = Σ src[x ..= x + 2r]`, so each output row is
/// `2r` samples shorter and output sample `x` is centred on input sample
/// `x + r`. `dst` is resized to fit. The taps are added as shifted slices,
/// which the compiler turns into vector additions.
fn box_rows<T: Copy + Default + AddAssign>(src: &[T], dst: &mut Vec<T>, width: usize) {
    let out_w = width + 1 - BOX_TAPS as usize;
    let rows = src.len() / width;
    dst.clear();
    dst.resize(out_w * rows, T::default());
    for (s, d) in src.chunks_exact(width).zip(dst.chunks_exact_mut(out_w)) {
        d.copy_from_slice(&s[..out_w]);
        for tap in 1..BOX_TAPS as usize {
            for (d, &v) in d.iter_mut().zip(&s[tap..]) {
                *d += v;
            }
        }
    }
}

/// One vertical box pass over `height` rows of `width` samples (at least
/// [`BOX_TAPS`] rows), like [`box_rows`] down the columns: the output has
/// `2r` rows fewer. `dst` is resized to fit.
fn box_columns<T: Copy + Default + AddAssign>(
    src: &[T],
    dst: &mut Vec<T>,
    width: usize,
    height: usize,
) {
    let taps = BOX_TAPS as usize;
    let out_h = height + 1 - taps;
    dst.clear();
    dst.resize(out_h * width, T::default());
    for (y, d) in dst.chunks_exact_mut(width).enumerate() {
        d.copy_from_slice(&src[y * width..(y + 1) * width]);
        for tap in 1..taps {
            let row = &src[(y + tap) * width..(y + tap + 1) * width];
            for (d, &v) in d.iter_mut().zip(row) {
                *d += v;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use blurrd_core::RectI;

    use super::InkPlanes;
    use crate::image::RgbView;

    fn rect(x0: i32, y0: i32, x1: i32, y1: i32) -> RectI {
        RectI::new(x0, y0, x1, y1).unwrap()
    }

    #[test]
    fn flat_images_have_no_ink() {
        let data = vec![90u8; 3 * 40 * 30];
        let view = RgbView::new(&data, 40, 30, 120).unwrap();
        let mut planes = InkPlanes::default();
        planes.compute(&view, rect(5, 5, 35, 25));
        assert_eq!(planes.count_above(rect(0, 0, 40, 30), 0), (0, 600));
        assert_eq!(
            planes.lmin_quantile_and_median(rect(5, 5, 35, 25), 0.97),
            Some((90, 90))
        );
    }

    #[test]
    fn a_thin_stroke_stands_out_in_any_channel() {
        // A one-pixel vertical line, bright in the blue channel only.
        let (w, h) = (40usize, 30usize);
        let mut data = vec![20u8; 3 * w * h];
        for y in 0..h {
            data[3 * (y * w + 20) + 2] = 220;
        }
        let view = RgbView::new(&data, w as u32, h as u32, 3 * w).unwrap();
        let mut planes = InkPlanes::default();
        planes.compute(&view, rect(8, 8, 32, 22));
        // The line keeps about 0.85 of its contrast against the
        // σ ≈ 2.45 blur; its neighbours far less.
        let mut columns = vec![];
        planes.column_counts(rect(8, 8, 32, 22), 120, &mut columns);
        let lit: Vec<usize> = (0..columns.len()).filter(|&i| columns[i] > 0).collect();
        assert_eq!(lit, vec![12]);
        assert_eq!(columns[12], 14);
        assert_eq!(
            planes.lmin_quantile_and_median(rect(8, 8, 32, 22), 0.97),
            Some((20, 20))
        );
        assert_eq!(
            planes.luma(20, 10),
            ((114 * 220 + 299 * 20 + 587 * 20 + 500) / 1000) as u8
        );
        assert_eq!(planes.luma(0, 0), 0);
    }

    #[test]
    fn box_passes_match_a_direct_sum() {
        let (w, h) = (9usize, 7usize);
        let src: Vec<u32> = (0..w * h).map(|i| (i * 37 % 101) as u32).collect();
        let narrow: Vec<u16> = src.iter().map(|&v| v as u16).collect();
        let mut dst16 = vec![];
        super::box_rows(&narrow, &mut dst16, w);
        assert!(dst16.iter().map(|&v| u32::from(v)).eq({
            let mut wide = vec![];
            super::box_rows(&src, &mut wide, w);
            wide
        }));
        let mut dst = vec![];
        super::box_rows(&src, &mut dst, w);
        assert_eq!(dst.len(), (w - 4) * h);
        for y in 0..h {
            for x in 0..w - 4 {
                let direct: u32 = (x..x + 5).map(|i| src[y * w + i]).sum();
                assert_eq!(dst[y * (w - 4) + x], direct);
            }
        }
        super::box_columns(&src, &mut dst, w, h);
        assert_eq!(dst.len(), w * (h - 4));
        for y in 0..h - 4 {
            for x in 0..w {
                let direct: u32 = (y..y + 5).map(|i| src[i * w + x]).sum();
                assert_eq!(dst[y * w + x], direct);
            }
        }
    }

    #[test]
    fn the_blur_matches_a_clamped_reference() {
        // Three 5-tap passes per direction with the crop's edge pixels
        // repeated, computed directly with the 13-tap kernel they add up
        // to, against the planes of an area at the crop's corner.
        let (w, h) = (23usize, 19usize);
        let data: Vec<u8> = (0..3 * w * h).map(|i| (i * 53 % 251) as u8).collect();
        let view = RgbView::new(&data, w as u32, h as u32, 3 * w).unwrap();
        let mut planes = InkPlanes::default();
        planes.compute(&view, rect(0, 0, 15, 12));
        let px = |x: i64, y: i64, c: usize| {
            let (x, y) = (x.clamp(0, w as i64 - 1), y.clamp(0, h as i64 - 1));
            i64::from(data[3 * (y as usize * w + x as usize) + c])
        };
        let mut kernel = vec![1i64];
        for _ in 0..3 {
            let mut next = vec![0i64; kernel.len() + 4];
            for (i, &k) in kernel.iter().enumerate() {
                for t in 0..5 {
                    next[i + t] += k;
                }
            }
            kernel = next;
        }
        let scale = 125i64 * 125;
        for y in 0..12i64 {
            for x in 0..15i64 {
                let mut ink = 0;
                for c in 0..3 {
                    let mut sum = 0;
                    for (j, &ky) in kernel.iter().enumerate() {
                        for (i, &kx) in kernel.iter().enumerate() {
                            sum += ky * kx * px(x + i as i64 - 6, y + j as i64 - 6, c);
                        }
                    }
                    let d = ((px(x, y, c) * scale - sum).abs() + scale / 2) / scale;
                    ink = ink.max(d.min(255));
                }
                assert_eq!(
                    i64::from(planes.ink_at(x as i32, y as i32)),
                    ink,
                    "({x}, {y})"
                );
            }
        }
    }
}
