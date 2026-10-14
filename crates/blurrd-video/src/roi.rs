//! Region-of-interest copies of decoded frames.
//!
//! Detection and the in-analysis verify work on two HUD crops per frame
//! instead of the full picture. A crop is copied out of the decoder's frame
//! (which goes back to the decoder right away) into an owned, reusable
//! [`YuvRoi`]. Its edges are aligned outward to even coordinates so its
//! chroma samples are exactly the full frame's, which is what lets the blur
//! run on the crop with bit-identical results (locality, see
//! `blurrd-blur`).

use blurrd_blur::{PlaneMut, YuvFrameMut};
use blurrd_core::RectI;

use crate::{PixelLayout, VideoError, VideoFrame};

/// How the samples of a [`YuvRoi`] are stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum RoiLayout {
    /// Planar 8-bit: Y, U, V planes.
    #[default]
    I420,
    /// Semi-planar 8-bit: Y plane and interleaved U/V plane.
    Nv12,
    /// Semi-planar 10-bit in 16-bit little-endian samples, value in the
    /// high bits. Planar 10-bit frames are converted to this layout.
    P010,
}

/// An owned copy of a rectangle of a frame, in one of the layouts the blur
/// works on. Reuse one per crop to avoid allocations.
#[derive(Debug, Clone, Default)]
pub struct YuvRoi {
    rect: Option<RectI>,
    layout: RoiLayout,
    /// Luma, `width` samples per row.
    y: Vec<u8>,
    /// I420: U plane; NV12/P010: interleaved U/V plane.
    c0: Vec<u8>,
    /// I420: V plane; unused otherwise.
    c1: Vec<u8>,
}

impl YuvRoi {
    /// An empty crop.
    pub fn new() -> Self {
        Self::default()
    }

    /// The part of the full frame this crop holds (even-aligned), or `None`
    /// before the first copy.
    pub fn rect(&self) -> Option<RectI> {
        self.rect
    }

    /// An 8-bit planar picture of `width × height` pixels (both even) that
    /// is not cut out of a frame, such as a downscaled copy of one. Its
    /// rectangle starts at `(0, 0)`.
    pub(crate) fn from_i420(
        width: u32,
        height: u32,
        y: Vec<u8>,
        u: Vec<u8>,
        v: Vec<u8>,
    ) -> Option<Self> {
        let rect = RectI::new(0, 0, width.cast_signed(), height.cast_signed()).ok()?;
        Some(Self {
            rect: Some(rect),
            layout: RoiLayout::I420,
            y,
            c0: u,
            c1: v,
        })
    }

    /// Sample layout.
    pub fn layout(&self) -> RoiLayout {
        self.layout
    }

    fn size(&self) -> (usize, usize) {
        self.rect
            .map_or((0, 0), |r| (r.width() as usize, r.height() as usize))
    }

    /// Bytes per sample.
    fn sample_bytes(&self) -> usize {
        if self.layout == RoiLayout::P010 { 2 } else { 1 }
    }

    /// Luma samples (8-bit layouts) or little-endian 16-bit samples (P010),
    /// `width` samples per row without padding.
    pub fn luma(&self) -> &[u8] {
        &self.y
    }

    /// Chroma planes: `(U, Some(V))` for I420, `(UV, None)` for the
    /// interleaved layouts. Rows hold `width / 2` chroma pixels.
    pub fn chroma(&self) -> (&[u8], Option<&[u8]>) {
        match self.layout {
            RoiLayout::I420 => (&self.c0, Some(&self.c1)),
            RoiLayout::Nv12 | RoiLayout::P010 => (&self.c0, None),
        }
    }

    /// A writable frame view of the crop, for the blur with
    /// `origin = (rect.x0, rect.y0)`. `None` before the first copy.
    pub fn as_frame_mut(&mut self) -> Option<YuvFrameMut<'_>> {
        let (w, h) = self.size();
        if w == 0 {
            return None;
        }
        let bytes = self.sample_bytes();
        let (cw, ch) = (w / 2, h / 2);

        Some(match self.layout {
            RoiLayout::I420 => YuvFrameMut::I420 {
                y: plane(&mut self.y, w, h, w),
                u: plane(&mut self.c0, cw, ch, cw),
                v: plane(&mut self.c1, cw, ch, cw),
            },
            RoiLayout::Nv12 => YuvFrameMut::Nv12 {
                y: plane(&mut self.y, w, h, w),
                uv: plane(&mut self.c0, cw, ch, 2 * cw),
            },
            RoiLayout::P010 => YuvFrameMut::P010 {
                y: plane(&mut self.y, w, h, w * bytes),
                uv: plane(&mut self.c0, cw, ch, 2 * cw * bytes),
            },
        })
    }
}

/// A plane view over the first `stride × height` bytes of `data`.
fn plane(data: &mut [u8], width: usize, height: usize, stride: usize) -> PlaneMut<'_> {
    PlaneMut {
        data: &mut data[..stride * height],
        stride,
        width: width as u32,
        height: height as u32,
    }
}

/// Aligns `roi` outward to even coordinates and clamps it to a frame of
/// `width × height` (both even for 4:2:0 video; an odd last column or row is
/// left out). `None` if nothing remains.
pub(crate) fn align_even(roi: RectI, width: u32, height: u32) -> Option<RectI> {
    let (w, h) = ((width & !1).cast_signed(), (height & !1).cast_signed());
    let x0 = (roi.x0.max(0) / 2) * 2;
    let y0 = (roi.y0.max(0) / 2) * 2;
    let x1 = (roi.x1.min(w) + 1) / 2 * 2;
    let y1 = (roi.y1.min(h) + 1) / 2 * 2;
    RectI::new(x0, y0, x1.min(w), y1.min(h)).ok()
}

impl VideoFrame {
    /// Copies the rectangle `roi` of this frame into `out`, aligned outward
    /// to even coordinates and clamped to the frame. Planar 10-bit frames
    /// are converted to P010 (`value << 6`) on the way.
    ///
    /// # Errors
    ///
    /// [`VideoError::Unsupported`] when `roi` does not overlap the frame or
    /// the frame's planes are not laid out as expected.
    pub fn roi_yuv(&self, roi: RectI, out: &mut YuvRoi) -> Result<(), VideoError> {
        let rect = align_even(roi, self.width(), self.height())
            .ok_or_else(|| VideoError::Unsupported(format!("crop {roi} is outside the frame")))?;
        let (x0, y0) = (rect.x0 as usize, rect.y0 as usize);
        let (w, h) = (rect.width() as usize, rect.height() as usize);
        let (cx0, cy0, cw, ch) = (x0 / 2, y0 / 2, w / 2, h / 2);
        let frame = self.raw();
        let plane = |index: usize| -> Result<(&[u8], usize), VideoError> {
            // SAFETY: reading the line size of a valid frame.
            let stride = unsafe { (*frame.as_ptr()).linesize[index] };
            let stride = usize::try_from(stride)
                .map_err(|_| VideoError::Unsupported(String::from("negative line size")))?;
            Ok((frame.data(index), stride))
        };
        match self.layout() {
            PixelLayout::I420 => {
                out.layout = RoiLayout::I420;
                copy_rows(plane(0)?, x0, y0, w, h, &mut out.y);
                copy_rows(plane(1)?, cx0, cy0, cw, ch, &mut out.c0);
                copy_rows(plane(2)?, cx0, cy0, cw, ch, &mut out.c1);
            }
            PixelLayout::Nv12 => {
                out.layout = RoiLayout::Nv12;
                copy_rows(plane(0)?, x0, y0, w, h, &mut out.y);
                copy_rows(plane(1)?, 2 * cx0, cy0, 2 * cw, ch, &mut out.c0);
                out.c1.clear();
            }
            PixelLayout::P010 => {
                out.layout = RoiLayout::P010;
                copy_rows(plane(0)?, 2 * x0, y0, 2 * w, h, &mut out.y);
                copy_rows(plane(1)?, 4 * cx0, cy0, 4 * cw, ch, &mut out.c0);
                out.c1.clear();
            }
            PixelLayout::I420P10 => {
                out.layout = RoiLayout::P010;
                copy_rows(plane(0)?, 2 * x0, y0, 2 * w, h, &mut out.y);
                shift_to_high_bits(&mut out.y);
                let (mut u, mut v) = (Vec::new(), Vec::new());
                copy_rows(plane(1)?, 2 * cx0, cy0, 2 * cw, ch, &mut u);
                copy_rows(plane(2)?, 2 * cx0, cy0, 2 * cw, ch, &mut v);
                out.c0.clear();
                out.c0.reserve(u.len() * 2);
                for (u, v) in u.as_chunks::<2>().0.iter().zip(v.as_chunks::<2>().0) {
                    let u = (u16::from_le_bytes(*u) << 6).to_le_bytes();
                    let v = (u16::from_le_bytes(*v) << 6).to_le_bytes();
                    out.c0.extend_from_slice(&[u[0], u[1], v[0], v[1]]);
                }
                out.c1.clear();
            }
        }
        out.rect = Some(rect);
        Ok(())
    }
}

/// Copies `rows` rows of `row_bytes` bytes, starting at byte `x` of row `y`
/// of the plane `src` (data, stride), into the packed `dst`.
fn copy_rows(
    src: (&[u8], usize),
    x: usize,
    y: usize,
    row_bytes: usize,
    rows: usize,
    dst: &mut Vec<u8>,
) {
    let (data, stride) = src;
    dst.clear();
    dst.reserve(row_bytes * rows);
    for row in 0..rows {
        let start = (y + row) * stride + x;
        dst.extend_from_slice(&data[start..start + row_bytes]);
    }
}

/// Converts 10-bit samples in the low bits to the P010 convention.
fn shift_to_high_bits(samples: &mut [u8]) {
    for pair in samples.as_chunks_mut::<2>().0 {
        *pair = (u16::from_le_bytes(*pair) << 6).to_le_bytes();
    }
}

#[cfg(test)]
mod tests {
    use blurrd_core::RectI;

    use super::align_even;

    fn rect(x0: i32, y0: i32, x1: i32, y1: i32) -> RectI {
        RectI::new(x0, y0, x1, y1).unwrap()
    }

    #[test]
    fn aligns_outward_and_clamps() {
        assert_eq!(
            align_even(rect(661, 81, 1919, 413), 1920, 1080),
            Some(rect(660, 80, 1920, 414))
        );
        assert_eq!(
            align_even(rect(-5, -3, 7, 9), 1920, 1080),
            Some(rect(0, 0, 8, 10))
        );
        assert_eq!(
            align_even(rect(1900, 1070, 2000, 1200), 1921, 1081),
            Some(rect(1900, 1070, 1920, 1080))
        );
        assert_eq!(align_even(rect(2000, 0, 2100, 10), 1920, 1080), None);
    }
}
