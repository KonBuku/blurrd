//! Views of YUV 4:2:0 frames and their planes.

use wide::bytemuck::cast;
use wide::{f32x8, i16x8, i32x8, u8x16, u16x8, u32x8};

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

    /// The planes the blur writes, each with the channels stored in it.
    ///
    /// The interleaved chroma plane of NV12 and P010 is one target with
    /// both chroma channels, so U and V are read and written together in a
    /// single pass over the plane.
    pub(crate) fn targets(&self) -> impl Iterator<Item = Target> + use<> {
        let luma = self.luma().stride;
        let targets = match self {
            Self::I420 { u, v, .. } => [
                Some(Target::new(0, luma, LayoutKind::Packed8, &[0])),
                Some(Target::new(1, u.stride, LayoutKind::Packed8, &[1])),
                Some(Target::new(2, v.stride, LayoutKind::Packed8, &[2])),
            ],
            Self::Nv12 { uv, .. } => [
                Some(Target::new(0, luma, LayoutKind::Packed8, &[0])),
                Some(Target::new(1, uv.stride, LayoutKind::Pair8, &[1, 2])),
                None,
            ],
            Self::P010 { uv, .. } => [
                Some(Target::new(0, luma, LayoutKind::Packed10, &[0])),
                Some(Target::new(1, uv.stride, LayoutKind::Pair10, &[1, 2])),
                None,
            ],
        };
        targets.into_iter().flatten()
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
        let chroma = match self {
            Self::I420 { u, v, .. } => [Some(("U", u, bytes)), Some(("V", v, bytes))],
            Self::Nv12 { uv, .. } | Self::P010 { uv, .. } => [Some(("UV", uv, 2 * bytes)), None],
        };
        for (name, plane, pixel_bytes) in chroma.into_iter().flatten() {
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

/// How the pixels of a target plane are stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LayoutKind {
    /// One 8-bit sample per pixel ([`Packed8`]).
    Packed8,
    /// One 16-bit P010 sample per pixel ([`Packed10`]).
    Packed10,
    /// An interleaved pair of 8-bit samples per pixel ([`Pair8`]).
    Pair8,
    /// An interleaved pair of 16-bit P010 samples per pixel ([`Pair10`]).
    Pair10,
}

/// One plane the blur writes and the channels (0 = Y, 1 = U, 2 = V) of
/// each of its pixels, in storage order.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Target {
    /// Index of the plane, for [`YuvFrameMut::plane_data`].
    pub(crate) plane: usize,
    /// Bytes from one row to the next.
    pub(crate) stride: usize,
    /// How the pixels are stored.
    pub(crate) layout: LayoutKind,
    /// The channels of each pixel.
    pub(crate) channels: &'static [usize],
}

impl Target {
    const fn new(
        plane: usize,
        stride: usize,
        layout: LayoutKind,
        channels: &'static [usize],
    ) -> Self {
        Self {
            plane,
            stride,
            layout,
            channels,
        }
    }

    /// Resolution of the plane relative to luma.
    pub(crate) const fn subsampling(&self) -> Subsampling {
        if self.plane == 0 {
            Subsampling::Luma
        } else {
            Subsampling::Chroma
        }
    }

    /// Largest sample value (255 for 8-bit, 1023 for 10-bit).
    pub(crate) const fn max_value(&self) -> f32 {
        match self.layout {
            LayoutKind::Packed8 | LayoutKind::Pair8 => 255.0,
            LayoutKind::Packed10 | LayoutKind::Pair10 => 1023.0,
        }
    }
}

// The 16-bit layouts reinterpret little-endian sample bytes as lanes.
#[cfg(not(target_endian = "little"))]
compile_error!("blurrd-blur reads 16-bit samples as little-endian vector lanes");

/// How the `N` samples of a pixel (one channel, or an interleaved U/V pair)
/// are stored in a plane row, for loops specialised per layout. Eight
/// consecutive pixels move through vector registers; the 16-bit layouts
/// reinterpret the row bytes as lanes instead of assembling them byte by
/// byte, and a pair layout reads and writes both channels at once.
pub(crate) trait Layout<const N: usize> {
    /// Bytes per pixel.
    const STEP: usize;
    /// Loads the eight pixels at the start of `chunk` (at least
    /// `8 * STEP` bytes) as code values, one vector per channel.
    fn load8(chunk: &[u8]) -> [f32x8; N];
    /// Stores eight pixels of in-range code values at the start of `chunk`
    /// (all `8 * STEP` bytes are written).
    fn store8(chunk: &mut [u8], values: [i32x8; N]);
    /// Loads the pixel at the start of `pixel`.
    fn load(pixel: &[u8]) -> [f32; N];
    /// Stores in-range code values into the pixel at the start of `pixel`.
    fn store(pixel: &mut [u8], values: [i32; N]);
}

/// 8-bit samples next to each other (luma, I420 chroma).
#[derive(Debug)]
pub(crate) struct Packed8;

/// 10-bit P010 samples next to each other (P010 luma).
#[derive(Debug)]
pub(crate) struct Packed10;

/// Interleaved U/V pairs of 8-bit samples (NV12 chroma).
#[derive(Debug)]
pub(crate) struct Pair8;

/// Interleaved U/V pairs of 10-bit P010 samples (P010 chroma).
#[derive(Debug)]
pub(crate) struct Pair10;

/// The first `M` bytes of `bytes` as an array.
fn take<const M: usize>(bytes: &[u8]) -> [u8; M] {
    let mut array = [0; M];
    array.copy_from_slice(&bytes[..M]);
    array
}

/// Narrows in-range (0..=1023) lanes to 16-bit words.
fn narrow16(values: i32x8) -> u16x8 {
    cast(i16x8::from_i32x8_saturate(values))
}

impl Layout<1> for Packed8 {
    const STEP: usize = 1;

    fn load8(chunk: &[u8]) -> [f32x8; 1] {
        let mut bytes = [0; 16];
        bytes[..8].copy_from_slice(&chunk[..8]);
        let words = u16x8::from_u8x16_low(u8x16::from(bytes));
        [i32x8::from_u16x8(words).round_float()]
    }

    fn store8(chunk: &mut [u8], [values]: [i32x8; 1]) {
        let narrow = i16x8::from_i32x8_saturate(values);
        let bytes = u8x16::narrow_i16x8(narrow, narrow).to_array();
        chunk[..8].copy_from_slice(&bytes[..8]);
    }

    fn load(pixel: &[u8]) -> [f32; 1] {
        [f32::from(pixel[0])]
    }

    fn store(pixel: &mut [u8], [value]: [i32; 1]) {
        pixel[0] = value as u8;
    }
}

impl Layout<1> for Packed10 {
    const STEP: usize = 2;

    fn load8(chunk: &[u8]) -> [f32x8; 1] {
        let words = cast::<[u8; 16], u16x8>(take(chunk)) >> 6_u32;
        [i32x8::from_u16x8(words).round_float()]
    }

    fn store8(chunk: &mut [u8], [values]: [i32x8; 1]) {
        let words = narrow16(values) << 6_u32;
        chunk[..16].copy_from_slice(&cast::<u16x8, [u8; 16]>(words));
    }

    fn load(pixel: &[u8]) -> [f32; 1] {
        [f32::from(u16::from_le_bytes([pixel[0], pixel[1]]) >> 6)]
    }

    fn store(pixel: &mut [u8], [value]: [i32; 1]) {
        pixel[..2].copy_from_slice(&((value as u16) << 6).to_le_bytes());
    }
}

impl Layout<2> for Pair8 {
    const STEP: usize = 2;

    fn load8(chunk: &[u8]) -> [f32x8; 2] {
        // Each 16-bit lane holds one pair: U in the low byte, V in the high.
        let words = cast::<[u8; 16], u16x8>(take(chunk));
        let u = words & u16x8::splat(0x00FF);
        let v = words >> 8_u32;
        [
            i32x8::from_u16x8(u).round_float(),
            i32x8::from_u16x8(v).round_float(),
        ]
    }

    fn store8(chunk: &mut [u8], [u, v]: [i32x8; 2]) {
        let words = narrow16(u) | (narrow16(v) << 8_u32);
        chunk[..16].copy_from_slice(&cast::<u16x8, [u8; 16]>(words));
    }

    fn load(pixel: &[u8]) -> [f32; 2] {
        [f32::from(pixel[0]), f32::from(pixel[1])]
    }

    fn store(pixel: &mut [u8], [u, v]: [i32; 2]) {
        pixel[0] = u as u8;
        pixel[1] = v as u8;
    }
}

impl Layout<2> for Pair10 {
    const STEP: usize = 4;

    fn load8(chunk: &[u8]) -> [f32x8; 2] {
        // Each 32-bit lane holds one pair: U in the low half, V in the
        // high half, each with its value in the top 10 bits.
        let lanes = cast::<[u8; 32], u32x8>(take(chunk));
        let u = (lanes & u32x8::splat(0xFFFF)) >> 6_u32;
        let v = lanes >> 22_u32;
        // Both are below 1024, so the signed conversion is exact.
        [
            cast::<u32x8, i32x8>(u).round_float(),
            cast::<u32x8, i32x8>(v).round_float(),
        ]
    }

    fn store8(chunk: &mut [u8], [u, v]: [i32x8; 2]) {
        let (u, v) = (cast::<i32x8, u32x8>(u), cast::<i32x8, u32x8>(v));
        let lanes = (u << 6_u32) | (v << 22_u32);
        chunk[..32].copy_from_slice(&cast::<u32x8, [u8; 32]>(lanes));
    }

    fn load(pixel: &[u8]) -> [f32; 2] {
        let [u] = Packed10::load(pixel);
        let [v] = Packed10::load(&pixel[2..]);
        [u, v]
    }

    fn store(pixel: &mut [u8], [u, v]: [i32; 2]) {
        Packed10::store(pixel, [u]);
        Packed10::store(&mut pixel[2..], [v]);
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
