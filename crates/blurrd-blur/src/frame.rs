//! Views of YUV 4:2:0 frames and their planes.

use crate::BlurError;

/// A writable view of one image plane.
///
/// Row `r` starts at byte `r * stride` of `data`. For the interleaved chroma
/// plane of NV12 and P010, one pixel is a U/V sample pair, so `width` counts
/// pairs, not samples.
#[derive(Debug)]
pub struct PlaneMut<'a> {
    /// Sample bytes, row after row.
    pub data: &'a mut [u8],
    /// Distance between the starts of two rows, in bytes.
    pub stride: usize,
    /// Width in pixels.
    pub width: u32,
    /// Height in rows.
    pub height: u32,
}

impl PlaneMut<'_> {
    /// A read-only view of the same plane.
    pub fn as_plane(&self) -> Plane<'_> {
        Plane {
            data: self.data,
            stride: self.stride,
            width: self.width,
            height: self.height,
        }
    }
}

/// A read-only view of one image plane, laid out like [`PlaneMut`].
#[derive(Debug, Clone, Copy)]
pub struct Plane<'a> {
    /// Sample bytes, row after row.
    pub data: &'a [u8],
    /// Distance between the starts of two rows, in bytes.
    pub stride: usize,
    /// Width in pixels.
    pub width: u32,
    /// Height in rows.
    pub height: u32,
}

/// How one sample is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SampleFormat {
    /// One byte per sample (8-bit video).
    U8,
    /// Two little-endian bytes per sample with the value in the high bits,
    /// as in P010 (10-bit video, value `<< 6`).
    U16Le,
}

impl SampleFormat {
    /// Bytes per sample.
    pub const fn bytes(self) -> usize {
        match self {
            Self::U8 => 1,
            Self::U16Le => 2,
        }
    }
}

/// A writable 4:2:0 frame (or a rectangular part of one).
///
/// The chroma planes have half the luma resolution, rounded up. The blur
/// works on the planes directly; it never converts to RGB.
#[derive(Debug)]
#[non_exhaustive]
pub enum YuvFrameMut<'a> {
    /// Planar 8-bit 4:2:0 (`yuv420p`): separate U and V planes.
    I420 {
        /// Luma plane.
        y: PlaneMut<'a>,
        /// Blue-difference chroma plane.
        u: PlaneMut<'a>,
        /// Red-difference chroma plane.
        v: PlaneMut<'a>,
    },
    /// Semi-planar 8-bit 4:2:0: one plane of interleaved U/V pairs.
    Nv12 {
        /// Luma plane.
        y: PlaneMut<'a>,
        /// Interleaved chroma plane (U first).
        uv: PlaneMut<'a>,
    },
    /// Semi-planar 10-bit 4:2:0 with 16-bit little-endian samples.
    P010 {
        /// Luma plane.
        y: PlaneMut<'a>,
        /// Interleaved chroma plane (U first).
        uv: PlaneMut<'a>,
    },
}

impl YuvFrameMut<'_> {
    /// Width of the luma plane in pixels.
    pub fn width(&self) -> u32 {
        self.luma().width
    }

    /// Height of the luma plane in pixels.
    pub fn height(&self) -> u32 {
        self.luma().height
    }

    /// How samples are stored in every plane of this frame.
    pub fn sample_format(&self) -> SampleFormat {
        match self {
            Self::I420 { .. } | Self::Nv12 { .. } => SampleFormat::U8,
            Self::P010 { .. } => SampleFormat::U16Le,
        }
    }

    /// The luma plane.
    pub fn luma(&self) -> &PlaneMut<'_> {
        match self {
            Self::I420 { y, .. } | Self::Nv12 { y, .. } | Self::P010 { y, .. } => y,
        }
    }

    /// The three channels (Y, U, V) and the plane each one lives in.
    pub(crate) fn channels(&self) -> [Channel; 3] {
        let format = self.sample_format();
        let bytes = format.bytes();
        let luma = Channel::new(0, 0, bytes, format, self.luma(), Subsampling::Luma);
        match self {
            Self::I420 { u, v, .. } => [
                luma,
                Channel::new(1, 0, bytes, format, u, Subsampling::Chroma),
                Channel::new(2, 0, bytes, format, v, Subsampling::Chroma),
            ],
            Self::Nv12 { uv, .. } | Self::P010 { uv, .. } => [
                luma,
                Channel::new(1, 0, 2 * bytes, format, uv, Subsampling::Chroma),
                Channel::new(1, bytes, 2 * bytes, format, uv, Subsampling::Chroma),
            ],
        }
    }

    /// The bytes of plane `index` (0 = Y, then the chroma planes in order).
    pub(crate) fn plane_data(&mut self, index: usize) -> &mut [u8] {
        match self {
            Self::I420 { y, u, v } => match index {
                0 => y.data,
                1 => u.data,
                _ => v.data,
            },
            Self::Nv12 { y, uv } | Self::P010 { y, uv } => {
                if index == 0 {
                    y.data
                } else {
                    uv.data
                }
            }
        }
    }

    /// Checks that the planes are consistent: chroma has half the luma size
    /// (rounded up), every row fits into its stride and every plane holds
    /// all of its rows.
    pub(crate) fn validate(&self) -> Result<(), BlurError> {
        let luma = self.luma();
        if luma.width == 0 || luma.height == 0 {
            return Err(BlurError::InvalidFrame(format!(
                "empty luma plane {}x{}",
                luma.width, luma.height
            )));
        }
        let (cw, ch) = (luma.width.div_ceil(2), luma.height.div_ceil(2));
        let bytes = self.sample_format().bytes();
        let mut planes: Vec<(&str, &PlaneMut<'_>, usize)> = vec![("Y", luma, bytes)];
        match self {
            Self::I420 { u, v, .. } => {
                planes.push(("U", u, bytes));
                planes.push(("V", v, bytes));
            }
            Self::Nv12 { uv, .. } | Self::P010 { uv, .. } => planes.push(("UV", uv, 2 * bytes)),
        }
        for &(name, plane, pixel_bytes) in &planes[1..] {
            if (plane.width, plane.height) != (cw, ch) {
                return Err(BlurError::InvalidFrame(format!(
                    "{name} plane is {}x{}, expected {cw}x{ch} for a {}x{} luma plane",
                    plane.width, plane.height, luma.width, luma.height
                )));
            }
            check_plane(name, plane, pixel_bytes)?;
        }
        check_plane("Y", luma, bytes)
    }
}

/// Checks that `plane` holds `height` rows of `width` pixels.
fn check_plane(name: &str, plane: &PlaneMut<'_>, pixel_bytes: usize) -> Result<(), BlurError> {
    let row = plane.width as usize * pixel_bytes;
    let needed = (plane.height as usize - 1) * plane.stride + row;
    if plane.stride < row || plane.data.len() < needed {
        return Err(BlurError::InvalidFrame(format!(
            "{name} plane: stride {} and {} bytes cannot hold {} rows of {row} bytes",
            plane.stride,
            plane.data.len(),
            plane.height
        )));
    }
    Ok(())
}

/// Resolution of a channel relative to the luma plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Subsampling {
    /// Full resolution.
    Luma,
    /// Half resolution in both directions.
    Chroma,
}

impl Subsampling {
    /// Luma pixels per channel pixel along each axis.
    pub(crate) const fn factor(self) -> i32 {
        match self {
            Self::Luma => 1,
            Self::Chroma => 2,
        }
    }
}

/// Where the samples of one channel (Y, U or V) live inside a plane.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Channel {
    /// Index of the plane, for [`YuvFrameMut::plane_data`].
    pub(crate) plane: usize,
    /// Byte offset of the channel's sample inside a pixel.
    offset: usize,
    /// Bytes from one pixel to the next.
    step: usize,
    /// Bytes from one row to the next.
    stride: usize,
    /// Sample storage.
    pub(crate) format: SampleFormat,
    /// Resolution relative to luma.
    pub(crate) subsampling: Subsampling,
}

impl Channel {
    const fn new(
        plane: usize,
        offset: usize,
        step: usize,
        format: SampleFormat,
        view: &PlaneMut<'_>,
        subsampling: Subsampling,
    ) -> Self {
        Self {
            plane,
            offset,
            step,
            stride: view.stride,
            format,
            subsampling,
        }
    }

    /// Largest sample value (255 for 8-bit, 1023 for 10-bit).
    pub(crate) const fn max_value(&self) -> f32 {
        match self.format {
            SampleFormat::U8 => 255.0,
            SampleFormat::U16Le => 1023.0,
        }
    }

    /// Byte index of the sample at `(x, y)` (plane coordinates).
    #[inline]
    const fn index(&self, x: usize, y: usize) -> usize {
        y * self.stride + x * self.step + self.offset
    }

    /// The sample at `(x, y)` as a code value.
    #[inline]
    pub(crate) fn read(&self, data: &[u8], x: usize, y: usize) -> f32 {
        let i = self.index(x, y);
        match self.format {
            SampleFormat::U8 => f32::from(data[i]),
            SampleFormat::U16Le => f32::from(u16::from_le_bytes([data[i], data[i + 1]]) >> 6),
        }
    }

    /// Writes a code value (already rounded and clamped) at `(x, y)`.
    #[inline]
    pub(crate) fn write(&self, data: &mut [u8], x: usize, y: usize, value: u16) {
        let i = self.index(x, y);
        match self.format {
            SampleFormat::U8 => data[i] = value as u8,
            SampleFormat::U16Le => {
                data[i..i + 2].copy_from_slice(&(value << 6).to_le_bytes());
            }
        }
    }
}

/// Reads the sample at `(x, y)` of a read-only plane as a number.
///
/// For [`SampleFormat::U16Le`] the raw 16-bit value is returned (the 10-bit
/// code value shifted left by 6); ratios such as the high-frequency residual
/// do not depend on that scale.
pub(crate) fn sample(plane: &Plane<'_>, format: SampleFormat, x: usize, y: usize) -> f64 {
    let i = y * plane.stride + x * format.bytes();
    match format {
        SampleFormat::U8 => f64::from(plane.data[i]),
        SampleFormat::U16Le => f64::from(u16::from_le_bytes([plane.data[i], plane.data[i + 1]])),
    }
}
