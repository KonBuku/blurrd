//! Writable frames for the render: a small pool of encoder-owned frames,
//! layout conversion and blur views.
//!
//! Decoder frames are reference counted and shared with the decoder, which
//! may still predict later frames from them. Writing into one would corrupt
//! every frame decoded from it afterwards, so a frame that gets blurred (or
//! needs another layout for the encoder) is first copied into a frame of
//! its own. Frames without boxes go to the encoder untouched.

use blurrd_blur::{PlaneMut, YuvFrameMut};
use ffmpeg_the_third::{ffi, format, frame};

use crate::{PixelLayout, VideoError, VideoFrame};

/// Reusable frames of one layout and size. A frame comes back to the pool
/// after it was sent to the encoder; it is handed out again once the
/// encoder has released its reference (FFmpeg reports it writable).
#[derive(Default)]
pub(crate) struct FramePool {
    free: Vec<frame::Video>,
}

impl std::fmt::Debug for FramePool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FramePool")
            .field("frames", &self.free.len())
            .finish()
    }
}

/// Frames a pool keeps at most; more are only needed while an encoder
/// holds many frames, and those are freed when they come back.
const POOL_CAPACITY: usize = 32;

impl FramePool {
    /// A writable frame of `layout` and the given size.
    fn take(&mut self, layout: PixelLayout, width: u32, height: u32) -> frame::Video {
        let format = format::Pixel::from(crate::encode::pixel_format(layout));
        let reusable = self.free.iter().position(|f| {
            f.format() == format
                && f.width() == width
                && f.height() == height
                // SAFETY: checking the reference counts of a valid frame.
                && unsafe { ffi::av_frame_is_writable(f.as_ptr().cast_mut()) } > 0
        });
        match reusable {
            Some(index) => self.free.swap_remove(index),
            None => frame::Video::new(format, width, height),
        }
    }

    /// Returns a frame (possibly still referenced by an encoder).
    pub(crate) fn give_back(&mut self, frame: VideoFrame) {
        if self.free.len() < POOL_CAPACITY {
            self.free.push(frame.into_raw());
        }
    }

    /// A private copy of `source` in `layout` (converting if needed), with
    /// the source's index, PTS and frame properties.
    pub(crate) fn copy(
        &mut self,
        source: &VideoFrame,
        layout: PixelLayout,
    ) -> Result<VideoFrame, VideoError> {
        let mut target = self.take(layout, source.width(), source.height());
        convert(source, layout, &mut target);
        // SAFETY: both frames are valid; only properties (timestamps,
        // flags, colour tags, side data) are copied.
        let ret = unsafe { ffi::av_frame_copy_props(target.as_mut_ptr(), source.raw().as_ptr()) };
        if ret < 0 {
            return Err(VideoError::encode("copying frame properties")(
                ffmpeg_the_third::Error::from(ret),
            ));
        }
        Ok(VideoFrame::wrap(
            source.index(),
            source.pts(),
            layout,
            target,
        ))
    }
}

impl VideoFrame {
    /// Whether nobody else holds a reference to the pixels.
    pub(crate) fn is_writable(&self) -> bool {
        // SAFETY: checking the reference counts of a valid frame.
        unsafe { ffi::av_frame_is_writable(self.raw().as_ptr().cast_mut()) > 0 }
    }

    /// Makes the pixels private (copying them into a pool frame if the
    /// decoder still references them) and returns a view the blur can write
    /// into. Planar 10-bit frames have no blur view; convert them to P010
    /// with [`FramePool::copy`] first.
    pub(crate) fn make_writable(
        &mut self,
        pool: &mut FramePool,
    ) -> Result<YuvFrameMut<'_>, VideoError> {
        if self.layout() == PixelLayout::I420P10 {
            return Err(VideoError::Unsupported(String::from(
                "planar 10-bit frames must be converted to P010 before blurring",
            )));
        }
        if !self.is_writable() {
            let copy = pool.copy(self, self.layout())?;
            let shared = std::mem::replace(self, copy);
            drop(shared);
        }
        Ok(self.planes_mut())
    }

    /// A blur view of the frame's planes. The frame must be writable and in
    /// I420, NV12 or P010.
    fn planes_mut(&mut self) -> YuvFrameMut<'_> {
        let layout = self.layout();
        let (width, height) = (self.width() as usize, self.height() as usize);
        let (cw, ch) = (width.div_ceil(2), height.div_ceil(2));
        // SAFETY: the pointer of the valid frame owned by `self`.
        let raw = unsafe { self.raw_mut().as_mut_ptr() };
        let plane = |index: usize, w: usize, h: usize| -> PlaneMut<'_> {
            // SAFETY: plane `index` of a valid, writable frame holds
            // `linesize × h` bytes; planes are separate allocations or
            // disjoint ranges of one, so the slices never overlap, and they
            // live as long as the `&mut self` borrow.
            unsafe {
                let stride = usize::try_from((*raw).linesize[index]).unwrap_or(0);
                PlaneMut {
                    data: std::slice::from_raw_parts_mut((*raw).data[index], stride * h),
                    stride,
                    width: w as u32,
                    height: h as u32,
                }
            }
        };
        match layout {
            PixelLayout::I420 => YuvFrameMut::I420 {
                y: plane(0, width, height),
                u: plane(1, cw, ch),
                v: plane(2, cw, ch),
            },
            PixelLayout::Nv12 => YuvFrameMut::Nv12 {
                y: plane(0, width, height),
                uv: plane(1, cw, ch),
            },
            PixelLayout::P010 | PixelLayout::I420P10 => YuvFrameMut::P010 {
                y: plane(0, width, height),
                uv: plane(1, cw, ch),
            },
        }
    }
}

/// How chroma samples are stored in a frame.
#[derive(Clone, Copy)]
enum Chroma {
    /// Separate U and V planes of 8-bit samples.
    Planar8,
    /// Separate U and V planes of 10-bit samples in the low bits of 16.
    Planar10,
    /// One plane of interleaved U/V 8-bit samples.
    Interleaved8,
    /// One plane of interleaved U/V samples in the high bits of 16.
    Interleaved16,
}

impl Chroma {
    fn of(layout: PixelLayout) -> Self {
        match layout {
            PixelLayout::I420 => Self::Planar8,
            PixelLayout::I420P10 => Self::Planar10,
            PixelLayout::Nv12 => Self::Interleaved8,
            PixelLayout::P010 => Self::Interleaved16,
        }
    }
}

/// A sample as a 16-bit value with the significant bits at the top (the
/// P010 convention), from any layout.
#[inline]
fn read16(row: &[u8], x: usize, chroma: Chroma) -> u16 {
    match chroma {
        Chroma::Planar8 | Chroma::Interleaved8 => u16::from(row[x]) << 8,
        Chroma::Planar10 => u16::from_le_bytes([row[2 * x], row[2 * x + 1]]) << 6,
        Chroma::Interleaved16 => u16::from_le_bytes([row[2 * x], row[2 * x + 1]]),
    }
}

/// Writes a 16-bit high-bits sample in the target layout's format (8-bit
/// targets round to nearest).
#[inline]
fn write16(row: &mut [u8], x: usize, value: u16, chroma: Chroma) {
    match chroma {
        Chroma::Planar8 | Chroma::Interleaved8 => {
            row[x] = (u32::from(value) + 0x80).min(0xffff).wrapping_shr(8) as u8;
        }
        Chroma::Planar10 => {
            row[2 * x..2 * x + 2].copy_from_slice(&(value >> 6).to_le_bytes());
        }
        Chroma::Interleaved16 => {
            // Keep only 10 significant bits, as P010 requires.
            row[2 * x..2 * x + 2].copy_from_slice(&(value & 0xffc0).to_le_bytes());
        }
    }
}

/// Copies or converts the pixels of `source` into `target`, a frame of the
/// same size in `layout`.
fn convert(source: &VideoFrame, layout: PixelLayout, target: &mut frame::Video) {
    let src = source.raw();
    if source.layout() == layout {
        // SAFETY: both frames are valid and have the same format and size.
        let ret = unsafe { ffi::av_frame_copy(target.as_mut_ptr(), src.as_ptr()) };
        debug_assert!(ret >= 0, "same format and size");
        return;
    }
    let (from, to) = (Chroma::of(source.layout()), Chroma::of(layout));
    let (width, height) = (source.width() as usize, source.height() as usize);
    let (cw, ch) = (width.div_ceil(2), height.div_ceil(2));

    // Luma: same sample format as the chroma of the layout, one channel.
    let (src_stride, dst_stride) = (src.stride(0), target.stride(0));
    for y in 0..height {
        let row_in = &src.data(0)[y * src_stride..];
        let row_out = &mut target.data_mut(0)[y * dst_stride..];
        for x in 0..width {
            write16(row_out, x, read16(row_in, x, from), to);
        }
    }

    // Chroma: read U and V for each chroma pixel, write them in the target
    // arrangement.
    let interleaved = |c: Chroma| matches!(c, Chroma::Interleaved8 | Chroma::Interleaved16);
    for y in 0..ch {
        let (u, v): (Vec<u16>, Vec<u16>) = if interleaved(from) {
            let row = &src.data(1)[y * src.stride(1)..];
            (0..cw)
                .map(|x| (read16(row, 2 * x, from), read16(row, 2 * x + 1, from)))
                .unzip()
        } else {
            let row_u = &src.data(1)[y * src.stride(1)..];
            let row_v = &src.data(2)[y * src.stride(2)..];
            (0..cw)
                .map(|x| (read16(row_u, x, from), read16(row_v, x, from)))
                .unzip()
        };
        if interleaved(to) {
            let stride = target.stride(1);
            let row = &mut target.data_mut(1)[y * stride..];
            for x in 0..cw {
                write16(row, 2 * x, u[x], to);
                write16(row, 2 * x + 1, v[x], to);
            }
        } else {
            for (plane, values) in [(1, &u), (2, &v)] {
                let stride = target.stride(plane);
                let row = &mut target.data_mut(plane)[y * stride..];
                for (x, &value) in values.iter().enumerate() {
                    write16(row, x, value, to);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use ffmpeg_the_third::{format, frame};

    use super::{FramePool, convert};
    use crate::{PixelLayout, VideoFrame};

    /// A frame of `layout` whose luma at (x, y) is `luma(x, y)` and whose
    /// chroma is U = 0x40.., V = 0xC0.. (8-bit values, scaled for 10-bit).
    fn pattern(layout: PixelLayout, w: u32, h: u32) -> VideoFrame {
        let pixel = format::Pixel::from(crate::encode::pixel_format(layout));
        let mut f = frame::Video::new(pixel, w, h);
        let mut src = frame::Video::new(format::Pixel::YUV420P, w, h);
        for y in 0..h as usize {
            let stride = src.stride(0);
            for x in 0..w as usize {
                src.data_mut(0)[y * stride + x] = (16 + (x * 7 + y * 3) % 220) as u8;
            }
        }
        for (plane, base) in [(1, 0x40u8), (2, 0xc0u8)] {
            let stride = src.stride(plane);
            for y in 0..h.div_ceil(2) as usize {
                for x in 0..w.div_ceil(2) as usize {
                    src.data_mut(plane)[y * stride + x] = base + (x % 16) as u8;
                }
            }
        }
        let source = VideoFrame::wrap(0, 0, PixelLayout::I420, src);
        convert(&source, layout, &mut f);
        VideoFrame::wrap(0, 0, layout, f)
    }

    #[test]
    fn conversions_round_trip_through_every_layout() {
        let reference = pattern(PixelLayout::I420, 34, 18);
        for layout in [
            PixelLayout::Nv12,
            PixelLayout::P010,
            PixelLayout::I420P10,
            PixelLayout::I420,
        ] {
            let converted = pattern(layout, 34, 18);
            let mut back = frame::Video::new(format::Pixel::YUV420P, 34, 18);
            convert(&converted, PixelLayout::I420, &mut back);
            for plane in 0..3 {
                let rows = if plane == 0 { 18 } else { 9 };
                let width = if plane == 0 { 34 } else { 17 };
                for y in 0..rows {
                    let a = &reference.raw().data(plane)[y * reference.raw().stride(plane)..];
                    let b = &back.data(plane)[y * back.stride(plane)..];
                    assert_eq!(a[..width], b[..width], "{layout:?} plane {plane} row {y}");
                }
            }
        }
    }

    #[test]
    fn ten_bit_samples_scale_exactly() {
        let p010 = pattern(PixelLayout::P010, 4, 2);
        // Luma 16 (8-bit) is 64 << 6 in P010.
        let raw = p010.raw();
        assert_eq!(
            u16::from_le_bytes([raw.data(0)[0], raw.data(0)[1]]),
            16 << 8
        );
        let planar = pattern(PixelLayout::I420P10, 4, 2);
        let raw = planar.raw();
        assert_eq!(
            u16::from_le_bytes([raw.data(0)[0], raw.data(0)[1]]),
            16 << 2
        );
    }

    #[test]
    fn shared_frames_are_copied_before_writing() {
        let mut pool = FramePool::default();
        let original = pattern(PixelLayout::Nv12, 16, 8);
        let mut shared = original.share().unwrap();
        assert!(!shared.is_writable());
        {
            let mut view = shared.make_writable(&mut pool).unwrap();
            let blurrd_blur::YuvFrameMut::Nv12 { y, .. } = &mut view else {
                panic!("NV12 view");
            };
            y.data[0] = 0;
        }
        assert_ne!(original.raw().data(0)[0], 0, "the original is untouched");
        assert_eq!(shared.raw().data(0)[0], 0);
        assert_eq!(shared.index(), original.index());
        // A frame nobody else holds is written in place.
        let mut own = pattern(PixelLayout::I420, 16, 8);
        let before = own.raw().data(0).as_ptr();
        own.make_writable(&mut pool).unwrap();
        assert_eq!(own.raw().data(0).as_ptr(), before);
        // Pool frames are reused once they are writable again.
        pool.give_back(shared);
        let copy = pool.copy(&original, PixelLayout::Nv12).unwrap();
        assert!(copy.is_writable());
        for y in 0..8 {
            let row = |f: &VideoFrame| f.raw().data(0)[y * f.raw().stride(0)..][..16].to_vec();
            assert_eq!(row(&copy), row(&original), "row {y}");
        }
    }
}
