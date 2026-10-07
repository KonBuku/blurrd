//! Views of YUV 4:2:0 frames and their planes.

use wide::{f32x8, i16x8, i32x8, u8x16, u16x8};

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
    const fn index(&self, x: usize, y: usize) -> usize {
        y * self.stride + x * self.step + self.offset
    }

    /// Byte index of the first sample of row `y` from column `x` (plane
    /// coordinates) and the bytes from one sample to the next.
    pub(crate) const fn row_start(&self, x: usize, y: usize) -> (usize, usize) {
        (self.index(x, y), self.step)
    }
}

/// How the samples of one channel are laid out in a plane row, for loops
/// specialised per layout. Eight consecutive samples move through a fixed
/// array and are widened and narrowed with vector instructions.
pub(crate) trait Lanes {
    /// Bytes from one sample to the next.
    const STEP: usize;
    /// Bytes covered by eight consecutive samples, from the first byte of
    /// the first to the last byte of the eighth.
    const SPAN: usize;
    /// Loads eight samples from the start of `chunk` (at least `SPAN`
    /// bytes) as code values.
    fn load8(chunk: &[u8]) -> f32x8;
    /// Stores eight code values (rounded, in range) at the start of
    /// `chunk`, leaving the bytes of other channels untouched.
    fn store8(chunk: &mut [u8], values: i32x8);
    /// Loads the sample at byte index `i`.
    fn load(data: &[u8], i: usize) -> f32;
    /// Stores a code value (rounded, in range) at byte index `i`.
    fn store(data: &mut [u8], i: usize, value: i32);
}

/// 8-bit samples next to each other (luma, I420 chroma).
#[derive(Debug)]
pub(crate) struct Packed8;

/// 8-bit samples interleaved with another channel (NV12 chroma).
#[derive(Debug)]
pub(crate) struct Paired8;

/// 10-bit P010 samples next to each other (P010 luma).
#[derive(Debug)]
pub(crate) struct Packed10;

/// 10-bit P010 samples interleaved with another channel (P010 chroma).
#[derive(Debug)]
pub(crate) struct Paired10;

/// Copies the first `N` bytes of `chunk` into a zero-padded array of `M`.
fn padded<const N: usize, const M: usize>(chunk: &[u8]) -> [u8; M] {
    let mut bytes = [0u8; M];
    bytes[..N].copy_from_slice(&chunk[..N]);
    bytes
}

/// Little-endian 16-bit words of 16 bytes.
fn words(bytes: [u8; 16]) -> u16x8 {
    u16x8::from(std::array::from_fn(|k| {
        u16::from_le_bytes([bytes[2 * k], bytes[2 * k + 1]])
    }))
}

/// The bytes of eight little-endian 16-bit words.
fn word_bytes(words: u16x8) -> [u8; 16] {
    let words = words.to_array();
    std::array::from_fn(|i| words[i / 2].to_le_bytes()[i % 2])
}

/// Narrows eight in-range values (0..=1023) to 16-bit words.
fn narrow16(values: i32x8) -> u16x8 {
    u16x8::from(
        i16x8::from_i32x8_saturate(values)
            .to_array()
            .map(|v| v as u16),
    )
}

impl Lanes for Packed8 {
    const STEP: usize = 1;
    const SPAN: usize = 8;

    fn load8(chunk: &[u8]) -> f32x8 {
        let bytes = u8x16::from(padded::<8, 16>(chunk));
        i32x8::from_u16x8(u16x8::from_u8x16_low(bytes)).round_float()
    }

    fn store8(chunk: &mut [u8], values: i32x8) {
        let narrow = i16x8::from_i32x8_saturate(values);
        let bytes = u8x16::narrow_i16x8(narrow, narrow).to_array();
        chunk[..8].copy_from_slice(&bytes[..8]);
    }

    fn load(data: &[u8], i: usize) -> f32 {
        f32::from(data[i])
    }

    fn store(data: &mut [u8], i: usize, value: i32) {
        data[i] = value as u8;
    }
}

impl Lanes for Paired8 {
    const STEP: usize = 2;
    const SPAN: usize = 15;

    fn load8(chunk: &[u8]) -> f32x8 {
        let samples = words(padded::<15, 16>(chunk)) & u16x8::splat(0x00FF);
        i32x8::from_u16x8(samples).round_float()
    }

    fn store8(chunk: &mut [u8], values: i32x8) {
        let old = words(padded::<15, 16>(chunk)) & u16x8::splat(0xFF00);
        let bytes = word_bytes(old | narrow16(values));
        chunk[..15].copy_from_slice(&bytes[..15]);
    }

    fn load(data: &[u8], i: usize) -> f32 {
        Packed8::load(data, i)
    }

    fn store(data: &mut [u8], i: usize, value: i32) {
        Packed8::store(data, i, value);
    }
}

impl Lanes for Packed10 {
    const STEP: usize = 2;
    const SPAN: usize = 16;

    fn load8(chunk: &[u8]) -> f32x8 {
        let samples = words(padded::<16, 16>(chunk)) >> 6_u32;
        i32x8::from_u16x8(samples).round_float()
    }

    fn store8(chunk: &mut [u8], values: i32x8) {
        chunk[..16].copy_from_slice(&word_bytes(narrow16(values) << 6_u32));
    }

    fn load(data: &[u8], i: usize) -> f32 {
        f32::from(u16::from_le_bytes([data[i], data[i + 1]]) >> 6)
    }

    fn store(data: &mut [u8], i: usize, value: i32) {
        data[i..i + 2].copy_from_slice(&((value as u16) << 6).to_le_bytes());
    }
}

impl Lanes for Paired10 {
    const STEP: usize = 4;
    const SPAN: usize = 30;

    fn load8(chunk: &[u8]) -> f32x8 {
        let bytes: [u8; 32] = padded::<30, 32>(chunk);
        f32x8::from(std::array::from_fn(|k| {
            f32::from(u16::from_le_bytes([bytes[4 * k], bytes[4 * k + 1]]) >> 6)
        }))
    }

    fn store8(chunk: &mut [u8], values: i32x8) {
        let mut bytes: [u8; 32] = padded::<30, 32>(chunk);
        let words = (narrow16(values) << 6_u32).to_array();
        for (k, word) in words.into_iter().enumerate() {
            bytes[4 * k..4 * k + 2].copy_from_slice(&word.to_le_bytes());
        }
        chunk[..30].copy_from_slice(&bytes[..30]);
    }

    fn load(data: &[u8], i: usize) -> f32 {
        Packed10::load(data, i)
    }

    fn store(data: &mut [u8], i: usize, value: i32) {
        Packed10::store(data, i, value);
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
