//! Downscaled copies of whole frames.
//!
//! Thumbnails, the quarter-scale view of the screen gates and the
//! scene-cut detector, and the frames of the preview proxy are area
//! averages of the decoded planes: every output sample is the mean of the
//! source samples its footprint covers, computed in the YUV domain (luma
//! and chroma separately), so no full-size RGB copy of the frame is ever
//! made. 10-bit frames are averaged at their native depth and rounded to
//! 8 bits.

use crate::rgb::{RgbConverter, RgbImage};
use crate::roi::YuvRoi;
use crate::{PixelLayout, VideoError, VideoFrame};

/// An 8-bit planar 4:2:0 picture (`width` and `height` even).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct I420Planes {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) y: Vec<u8>,
    pub(crate) u: Vec<u8>,
    pub(crate) v: Vec<u8>,
}

impl VideoFrame {
    /// A `width × height` RGB copy of the whole frame (both rounded down
    /// to even numbers, at least 2), each pixel the area average of the
    /// source pixels it covers, converted with `converter`.
    ///
    /// # Errors
    ///
    /// [`VideoError::Unsupported`] when the frame's planes are not laid out
    /// as expected.
    pub fn thumbnail(
        &self,
        width: u32,
        height: u32,
        converter: &RgbConverter,
        out: &mut RgbImage,
    ) -> Result<(), VideoError> {
        let planes = self.downscale(width, height)?;
        let roi = YuvRoi::from_i420(planes.width, planes.height, planes.y, planes.u, planes.v)
            .ok_or_else(|| VideoError::Unsupported(String::from("empty thumbnail")))?;
        converter.convert(&roi, out)
    }

    /// The frame area-averaged to `width × height` 8-bit I420 (sizes
    /// rounded down to even numbers, at least 2).
    pub(crate) fn downscale(&self, width: u32, height: u32) -> Result<I420Planes, VideoError> {
        let (ow, oh) = ((width & !1).max(2), (height & !1).max(2));
        let (sw, sh) = (self.width() as usize, self.height() as usize);
        let frame = self.raw();
        let plane = |index: usize| -> Result<(&[u8], usize), VideoError> {
            // SAFETY: reading the line size of the valid frame owned by
            // `self`.
            let stride = unsafe { (*frame.as_ptr()).linesize[index] };
            let stride = usize::try_from(stride)
                .map_err(|_| VideoError::Unsupported(String::from("negative line size")))?;
            Ok((frame.data(index), stride))
        };
        let luma = Grid::new(sw, sh, ow as usize, oh as usize);
        let chroma = Grid::new(sw / 2, sh / 2, ow as usize / 2, oh as usize / 2);
        let mut out = I420Planes {
            width: ow,
            height: oh,
            ..I420Planes::default()
        };
        match self.layout() {
            PixelLayout::I420 => {
                luma.average(plane(0)?, Samples::U8(1, 0), 1, &mut out.y);
                chroma.average(plane(1)?, Samples::U8(1, 0), 1, &mut out.u);
                chroma.average(plane(2)?, Samples::U8(1, 0), 1, &mut out.v);
            }
            PixelLayout::Nv12 => {
                luma.average(plane(0)?, Samples::U8(1, 0), 1, &mut out.y);
                chroma.average(plane(1)?, Samples::U8(2, 0), 1, &mut out.u);
                chroma.average(plane(1)?, Samples::U8(2, 1), 1, &mut out.v);
            }
            PixelLayout::I420P10 => {
                luma.average(plane(0)?, Samples::U16(1, 0), 4, &mut out.y);
                chroma.average(plane(1)?, Samples::U16(1, 0), 4, &mut out.u);
                chroma.average(plane(2)?, Samples::U16(1, 0), 4, &mut out.v);
            }
            PixelLayout::P010 => {
                luma.average(plane(0)?, Samples::U16(1, 0), 256, &mut out.y);
                chroma.average(plane(1)?, Samples::U16(2, 0), 256, &mut out.u);
                chroma.average(plane(1)?, Samples::U16(2, 1), 256, &mut out.v);
            }
        }
        Ok(out)
    }
}

/// Where the samples of one plane are in its rows: `(step, offset)` in
/// samples (sample `x` is element `x × step + offset`), 8-bit or 16-bit
/// little-endian.
#[derive(Debug, Clone, Copy)]
enum Samples {
    U8(usize, usize),
    U16(usize, usize),
}

impl Samples {
    /// Adds the samples of `row` to `sums` (one per sample).
    fn accumulate(self, row: &[u8], sums: &mut [u32]) {
        match self {
            Self::U8(1, _) => {
                for (sum, &value) in sums.iter_mut().zip(row) {
                    *sum += u32::from(value);
                }
            }
            Self::U8(step, offset) => {
                for (sum, chunk) in sums.iter_mut().zip(row[offset..].chunks(step)) {
                    *sum += u32::from(chunk[0]);
                }
            }
            Self::U16(step, offset) => {
                for (sum, chunk) in sums.iter_mut().zip(row[2 * offset..].chunks(2 * step)) {
                    *sum += u32::from(u16::from_le_bytes([chunk[0], chunk[1]]));
                }
            }
        }
    }
}

/// The source footprints of the output samples of one plane.
struct Grid {
    /// Source samples per row.
    width: usize,
    /// Per output column, the source columns `start..end` it averages.
    columns: Vec<(usize, usize)>,
    /// Per output row, the source rows `start..end` it averages.
    rows: Vec<(usize, usize)>,
}

impl Grid {
    /// Footprints for scaling `source_w × source_h` samples to
    /// `out_w × out_h`. Every footprint holds at least one sample (an
    /// enlarged plane repeats samples).
    fn new(source_w: usize, source_h: usize, out_w: usize, out_h: usize) -> Self {
        Self {
            width: source_w.max(1),
            columns: footprints(source_w, out_w),
            rows: footprints(source_h, out_h),
        }
    }

    /// Averages the plane `(data, stride)` with its `samples` into `out`
    /// (one byte per output sample); `scale` maps the summed values to 8
    /// bits (1 for 8-bit, 4 for 10-bit values in the low bits, 256 for
    /// 10-bit values in the high bits). The rows of a footprint are summed
    /// per column first, so the inner loop runs over contiguous samples.
    fn average(
        &self,
        (data, stride): (&[u8], usize),
        samples: Samples,
        scale: u32,
        out: &mut Vec<u8>,
    ) {
        out.clear();
        out.reserve(self.columns.len() * self.rows.len());
        let mut sums = vec![0u32; self.width];
        for &(y0, y1) in &self.rows {
            sums.fill(0);
            for y in y0..y1 {
                samples.accumulate(&data[y * stride..], &mut sums);
            }
            let rows = (y1 - y0) as u32;
            for &(x0, x1) in &self.columns {
                let sum: u32 = sums[x0..x1].iter().sum();
                let divisor = rows * (x1 - x0) as u32 * scale;
                out.push(((sum + divisor / 2) / divisor).min(255) as u8);
            }
        }
    }
}

/// Splits `source` samples into `out` consecutive non-empty runs, as
/// evenly as possible.
fn footprints(source: usize, out: usize) -> Vec<(usize, usize)> {
    let source = source.max(1);
    (0..out)
        .map(|i| {
            let start = (i * source / out).min(source - 1);
            let end = ((i + 1) * source / out).clamp(start + 1, source);
            (start, end)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::footprints;

    #[test]
    fn footprints_cover_the_source_once() {
        assert_eq!(footprints(8, 2), [(0, 4), (4, 8)]);
        assert_eq!(footprints(1920, 480)[479], (1916, 1920));
        let odd = footprints(1280, 426);
        assert_eq!(odd[0].0, 0);
        assert_eq!(odd[425].1, 1280);
        assert!(odd.windows(2).all(|w| w[0].1 == w[1].0));
        // Enlarging repeats samples.
        assert_eq!(footprints(2, 4), [(0, 1), (0, 1), (1, 2), (1, 2)]);
    }
}
