//! Bright text strokes of an RGB area: pixels whose white top-hat is high
//! in any channel.
//!
//! The result equals a per-channel [`crate::morph::white_tophat`] of the
//! area as a crop of its own (pixels outside it ignored), but the three
//! channels are filtered together on the interleaved rows: one pass over
//! `3 × width` bytes instead of three channel copies and three filters,
//! and each 5-tap horizontal extremum takes three element-wise operations
//! instead of four. The text chat runs this on every frame with a chat
//! box, so it is the detector's main cost.

use blurrd_core::RectI;

use crate::image::RgbView;

/// Half the kernel size: the kernel is `2 × RADIUS + 1` pixels square,
/// anchored at its centre.
const RADIUS: usize = 2;
/// Bytes between horizontally neighbouring samples of one channel.
const CHANNELS: usize = 3;
/// Padding on each side of a row buffer, in bytes.
const PAD: usize = RADIUS * CHANNELS;

#[derive(Clone, Copy)]
enum Op {
    Min,
    Max,
}

impl Op {
    /// The value outside the area that leaves the result unchanged.
    const fn neutral(self) -> u8 {
        match self {
            Self::Min => u8::MAX,
            Self::Max => 0,
        }
    }

    /// `out[i] = op(a[i], b[i])`.
    fn combine(self, out: &mut [u8], a: &[u8], b: &[u8]) {
        let pairs = out.iter_mut().zip(a.iter().zip(b));
        match self {
            Self::Min => pairs.for_each(|(o, (x, y))| *o = (*x).min(*y)),
            Self::Max => pairs.for_each(|(o, (x, y))| *o = (*x).max(*y)),
        }
    }

    /// `acc[i] = op(acc[i], b[i])`.
    fn apply(self, acc: &mut [u8], b: &[u8]) {
        let pairs = acc.iter_mut().zip(b);
        match self {
            Self::Min => pairs.for_each(|(a, y)| *a = (*a).min(*y)),
            Self::Max => pairs.for_each(|(a, y)| *a = (*a).max(*y)),
        }
    }
}

/// Row buffers of one filter pass.
#[derive(Debug, Clone, Default)]
struct RowBuffers {
    /// Result of the horizontal pass.
    horizontal: Vec<u8>,
    /// One padded row and its pairwise extrema.
    row: Vec<u8>,
    pairs: Vec<u8>,
    quads: Vec<u8>,
}

/// Reusable buffers.
#[derive(Debug, Clone, Default)]
pub(super) struct InkScratch {
    /// The area's pixels, `3 × width` bytes per row.
    src: Vec<u8>,
    eroded: Vec<u8>,
    opened: Vec<u8>,
    /// 1 where a channel's top-hat exceeds the threshold.
    hot: Vec<u8>,
    rows: RowBuffers,
}

impl InkScratch {
    /// Sets `out` (one byte per pixel of `area`, row after row) to 1 where
    /// the white top-hat of some channel exceeds `threshold`, else to 0.
    /// `area` is in reference pixels, `origin` is the reference position of
    /// the crop's top-left pixel; the area must lie inside the crop.
    pub(super) fn stroke_mask(
        &mut self,
        crop: &RgbView<'_>,
        origin: (i32, i32),
        area: RectI,
        threshold: u8,
        out: &mut Vec<u8>,
    ) {
        let stride = CHANNELS * area.width() as usize;
        self.src.clear();
        let x0 = CHANNELS * (area.x0 - origin.0) as usize;
        for y in area.y0..area.y1 {
            self.src
                .extend_from_slice(&crop.row((y - origin.1) as u32)[x0..x0 + stride]);
        }
        // Opening: erosion, then dilation of the erosion.
        filter(Op::Min, &self.src, stride, &mut self.rows, &mut self.eroded);
        filter(
            Op::Max,
            &self.eroded,
            stride,
            &mut self.rows,
            &mut self.opened,
        );
        // Compare every byte first (vectorises), then join the channels.
        self.hot.clear();
        self.hot.extend(
            self.src
                .iter()
                .zip(&self.opened)
                .map(|(&s, &o)| u8::from(s.saturating_sub(o) > threshold)),
        );
        out.clear();
        out.extend(
            self.hot
                .as_chunks::<CHANNELS>()
                .0
                .iter()
                .map(|[r, g, b]| r | g | b),
        );
    }
}

/// Applies the `(2 × RADIUS + 1)²` rectangle filter `op` to `input`, rows
/// of `stride` bytes, into `out`.
fn filter(op: Op, input: &[u8], stride: usize, bufs: &mut RowBuffers, out: &mut Vec<u8>) {
    let h = input.len() / stride;
    // Horizontal pass, three operations per byte: pairs[i] = op(row[i],
    // row[i + 3]), quads[i] = op(pairs[i], pairs[i + 6]), result[i] =
    // op(quads[i], row[i + 12]), where row is the input row padded with
    // the neutral value.
    bufs.horizontal.clear();
    bufs.horizontal.resize(stride * h, 0);
    let len = stride + 2 * PAD;
    bufs.row.clear();
    bufs.row.resize(len, op.neutral());
    bufs.pairs.resize(len - CHANNELS, 0);
    bufs.quads.resize(len - 3 * CHANNELS, 0);
    for (src, dst) in input
        .chunks_exact(stride)
        .zip(bufs.horizontal.chunks_exact_mut(stride))
    {
        bufs.row[PAD..PAD + stride].copy_from_slice(src);
        let row = &bufs.row;
        op.combine(&mut bufs.pairs, &row[..len - CHANNELS], &row[CHANNELS..]);
        op.combine(
            &mut bufs.quads,
            &bufs.pairs[..len - 3 * CHANNELS],
            &bufs.pairs[2 * CHANNELS..],
        );
        op.combine(dst, &bufs.quads[..stride], &row[4 * CHANNELS..]);
    }
    // Vertical pass: rows outside the area are skipped.
    out.clear();
    out.resize(stride * h, 0);
    for (y, dst) in out.chunks_exact_mut(stride).enumerate() {
        let first = y.saturating_sub(RADIUS);
        let last = (y + RADIUS + 1).min(h);
        dst.copy_from_slice(&bufs.horizontal[first * stride..(first + 1) * stride]);
        for r in first + 1..last {
            op.apply(dst, &bufs.horizontal[r * stride..(r + 1) * stride]);
        }
    }
}

#[cfg(test)]
mod tests {
    use blurrd_core::RectI;

    use super::InkScratch;
    use crate::image::{GrayImage, RgbView};
    use crate::morph::{MorphScratch, white_tophat};

    #[test]
    fn equals_a_tophat_of_each_channel() {
        // A noisy image with bright strokes, and an area that touches its
        // borders on two sides only.
        let (w, h) = (61u32, 37u32);
        let mut data = vec![0u8; (3 * w * h) as usize];
        let mut state = 0x2545_f491_u32;
        for v in &mut data {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            *v = (state % 160) as u8 + if state.is_multiple_of(9) { 90 } else { 0 };
        }
        let crop = RgbView::new(&data, w, h, 3 * w as usize).unwrap();
        let area = RectI::new(10, 107, 61, 137).unwrap();
        let origin = (0, 100);
        let mut mask = vec![];
        InkScratch::default().stroke_mask(&crop, origin, area, 40, &mut mask);

        let (aw, ah) = (area.width(), area.height());
        let mut expected = vec![0u8; (aw * ah) as usize];
        let mut tophat = GrayImage::default();
        let mut scratch = MorphScratch::default();
        for c in 0..3 {
            let mut channel = GrayImage::new(aw, ah);
            for y in 0..ah {
                for x in 0..aw {
                    let px = crop.pixel(x + 10, y + 7);
                    channel.data_mut()[(y * aw + x) as usize] = px[c];
                }
            }
            white_tophat(&channel, 5, 5, &mut tophat, &mut scratch);
            for (e, &t) in expected.iter_mut().zip(tophat.data()) {
                *e |= u8::from(t > 40);
            }
        }
        assert_eq!(mask, expected);
        assert!(mask.contains(&1) && mask.contains(&0));
    }
}
