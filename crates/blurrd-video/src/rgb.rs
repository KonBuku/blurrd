//! Converting YUV crops to 8-bit RGB for the detectors and OCR.
//!
//! SDR video is converted exactly like the reference the detector
//! thresholds were tuned on, FFmpeg's default `-pix_fmt bgr24` conversion:
//! the tagged matrix and range to full-range RGB, each chroma sample used
//! for its 2×2 luma block, with swscale's truncation (bit-identical on real
//! recordings). 10-bit SDR samples are rounded to 8 bits first.
//!
//! HDR video (PQ or HLG) is tone-mapped to SDR BT.709: linear light from
//! the transfer function, a luminance-preserving extended Reinhard curve
//! that maps SDR reference white (203 nits) to about 0.5 and a peak of
//! 1000 nits to 1.0, a BT.2020 to BT.709 gamut conversion with clipping,
//! and the BT.1886 (gamma 2.4) encoding.

#![allow(
    clippy::many_single_char_names,
    reason = "colour maths uses the conventional names y, u, v, r, g, b"
)]

use blurrd_blur::{SampleRange, YuvMatrix};

use crate::VideoError;
use crate::colour::{ColourTransfer, VideoColour};
use crate::roi::{RoiLayout, YuvRoi};

/// SDR reference white in nits (ITU-R BT.2408).
const REFERENCE_WHITE_NITS: f32 = 203.0;
/// Assumed mastering peak in nits for the tone curve.
const PEAK_NITS: f32 = 1000.0;
/// Entries of the transfer function look-up tables.
const LUT_SIZE: usize = 4096;

/// A packed 8-bit RGB image (`R, G, B` per pixel, no row padding).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RgbImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `3 × width × height` bytes, row after row.
    pub data: Vec<u8>,
}

/// SDR conversion tables, one entry per 8-bit code value.
///
/// They reproduce FFmpeg's swscale `yuv420p` to `bgr24` conversion bit for
/// bit (checked on real recordings): the luma term and each chroma term are
/// truncated separately, then added and clamped. That is the reference the
/// detector thresholds were tuned on; it is up to about two code values
/// darker than the exactly rounded conversion.
#[derive(Debug, Clone)]
struct SdrTables {
    y: [i16; 256],
    r_v: [i16; 256],
    g_u: [i16; 256],
    g_v: [i16; 256],
    b_u: [i16; 256],
}

impl SdrTables {
    fn new(matrix: YuvMatrix, range: SampleRange) -> Self {
        let (kr, kb) = weights(matrix);
        let kg = 1.0 - kr - kb;
        let (y_offset, y_scale, c_scale) = match range {
            SampleRange::Full => (0.0, 1.0, 1.0),
            _ => (16.0, 255.0 / 219.0, 255.0 / 224.0),
        };
        let table = |f: &dyn Fn(f64) -> f64| -> [i16; 256] {
            std::array::from_fn(|code| f(code as f64).floor() as i16)
        };
        Self {
            y: table(&|y| (y - y_offset) * y_scale),
            r_v: table(&|v| 2.0 * (1.0 - kr) * c_scale * (v - 128.0)),
            g_u: table(&|u| -2.0 * kb * (1.0 - kb) / kg * c_scale * (u - 128.0)),
            g_v: table(&|v| -2.0 * kr * (1.0 - kr) / kg * c_scale * (v - 128.0)),
            b_u: table(&|u| 2.0 * (1.0 - kb) * c_scale * (u - 128.0)),
        }
    }

    #[inline]
    fn rgb(&self, y: u8, u: u8, v: u8) -> [u8; 3] {
        let (y, u, v) = (usize::from(y), usize::from(u), usize::from(v));
        let luma = self.y[y];
        let clamp = |x: i16| x.clamp(0, 255) as u8;
        [
            clamp(luma + self.r_v[v]),
            clamp(luma + self.g_u[u] + self.g_v[v]),
            clamp(luma + self.b_u[u]),
        ]
    }
}

impl SdrTables {
    /// Converts an 8-bit crop `w` pixels wide (even): `luma` packed rows,
    /// chroma as `(U, Some(V))` planes or one interleaved `(UV, None)`
    /// plane of `w / 2` samples per row, each used for a 2 × 2 block. The
    /// same arithmetic as [`SdrTables::rgb`], one chroma lookup per pixel
    /// pair (the hot path of every analysed frame).
    fn convert_8bit(
        &self,
        luma: &[u8],
        (c0, c1): (&[u8], Option<&[u8]>),
        w: usize,
        out: &mut [u8],
    ) {
        let cw = w / 2;
        let clamp = |x: i16| x.clamp(0, 255) as u8;
        let pair = |px: &mut [u8; 6], ys: [u8; 2], u: u8, v: u8| {
            let (u, v) = (usize::from(u), usize::from(v));
            let (r, g, b) = (self.r_v[v], self.g_u[u] + self.g_v[v], self.b_u[u]);
            let [y0, y1] = ys.map(|y| self.y[usize::from(y)]);
            *px = [
                clamp(y0 + r),
                clamp(y0 + g),
                clamp(y0 + b),
                clamp(y1 + r),
                clamp(y1 + g),
                clamp(y1 + b),
            ];
        };
        for (row, (line, ys)) in out
            .chunks_exact_mut(3 * w)
            .zip(luma.chunks_exact(w))
            .enumerate()
        {
            let crow = row / 2;
            let pixels = line
                .as_chunks_mut::<6>()
                .0
                .iter_mut()
                .zip(ys.as_chunks::<2>().0);
            if let Some(c1) = c1 {
                let (us, vs) = (&c0[crow * cw..][..cw], &c1[crow * cw..][..cw]);
                for (((px, &ys), &u), &v) in pixels.zip(us).zip(vs) {
                    pair(px, ys, u, v);
                }
            } else {
                let uvs = &c0[crow * 2 * cw..][..2 * cw];
                for ((px, &ys), &[u, v]) in pixels.zip(uvs.as_chunks::<2>().0) {
                    pair(px, ys, u, v);
                }
            }
        }
    }
}

/// `(Kr, Kb)` of a matrix.
fn weights(matrix: YuvMatrix) -> (f64, f64) {
    match matrix {
        YuvMatrix::Bt601 => (0.299, 0.114),
        YuvMatrix::Bt2020 => (0.2627, 0.0593),
        _ => (0.2126, 0.0722),
    }
}

/// Tables for HDR tone mapping.
#[derive(Debug, Clone)]
struct HdrTables {
    /// Signal (0..=1, `LUT_SIZE` steps) to linear light in nits.
    eotf: Vec<f32>,
    /// Linear SDR light (0..=1) to the 8-bit BT.1886 code value.
    oetf: Vec<u8>,
    hlg: bool,
    wide_gamut: bool,
}

impl HdrTables {
    fn new(transfer: ColourTransfer, wide_gamut: bool) -> Self {
        let hlg = transfer == ColourTransfer::Hlg;
        let step = |i: usize| i as f32 / (LUT_SIZE - 1) as f32;
        let eotf = (0..LUT_SIZE)
            .map(|i| {
                if hlg {
                    hlg_inverse_oetf(step(i))
                } else {
                    pq_eotf(step(i))
                }
            })
            .collect();
        let oetf = (0..LUT_SIZE)
            .map(|i| (step(i).powf(1.0 / 2.4) * 255.0).round().clamp(0.0, 255.0) as u8)
            .collect();
        Self {
            eotf,
            oetf,
            hlg,
            wide_gamut,
        }
    }

    #[inline]
    fn lookup(table: &[f32], x: f32) -> f32 {
        table[(x.clamp(0.0, 1.0) * (LUT_SIZE - 1) as f32).round() as usize]
    }

    /// Normalised non-linear BT.2020 R'G'B' to 8-bit SDR RGB.
    fn map(&self, rgb: [f32; 3]) -> [u8; 3] {
        let mut lin = rgb.map(|c| Self::lookup(&self.eotf, c));
        if self.hlg {
            // HLG: scene light (0..=1) to display light with the BT.2100
            // OOTF for a 1000-nit display (system gamma 1.2).
            let ys = 0.2627 * lin[0] + 0.6780 * lin[1] + 0.0593 * lin[2];
            let gain = PEAK_NITS * ys.max(1e-6).powf(0.2);
            lin = lin.map(|c| c * gain);
        }
        // Luminance-preserving extended Reinhard tone curve.
        let white = PEAK_NITS / REFERENCE_WHITE_NITS;
        let rel = lin.map(|c| c / REFERENCE_WHITE_NITS);
        let l = 0.2627 * rel[0] + 0.6780 * rel[1] + 0.0593 * rel[2];
        let mapped = l * (1.0 + l / (white * white)) / (1.0 + l);
        let scale = if l > 1e-6 { mapped / l } else { 0.0 };
        let mut sdr = rel.map(|c| c * scale);
        if self.wide_gamut {
            let [r, g, b] = sdr;
            // BT.2087 conversion matrix (linear BT.2020 to BT.709).
            sdr = [
                1.660_491 * r - 0.587_641 * g - 0.072_850 * b,
                -0.124_550 * r + 1.132_9 * g - 0.008_349 * b,
                -0.018_151 * r - 0.100_579 * g + 1.118_73 * b,
            ];
        }
        sdr.map(|c| self.oetf[(c.clamp(0.0, 1.0) * (LUT_SIZE - 1) as f32).round() as usize])
    }
}

/// SMPTE ST 2084 EOTF: signal (0..=1) to nits.
fn pq_eotf(e: f32) -> f32 {
    const M1: f32 = 2610.0 / 16384.0;
    const M2: f32 = 2523.0 / 4096.0 * 128.0;
    const C1: f32 = 3424.0 / 4096.0;
    const C2: f32 = 2413.0 / 4096.0 * 32.0;
    const C3: f32 = 2392.0 / 4096.0 * 32.0;
    let p = e.max(0.0).powf(1.0 / M2);
    10000.0 * ((p - C1).max(0.0) / (C2 - C3 * p)).powf(1.0 / M1)
}

/// ARIB STD-B67 inverse OETF: signal (0..=1) to relative scene light.
fn hlg_inverse_oetf(e: f32) -> f32 {
    const A: f32 = 0.178_832_77;
    const B: f32 = 0.284_668_92;
    const C: f32 = 0.559_910_7;
    if e <= 0.5 {
        e * e / 3.0
    } else {
        (((e - C) / A).exp() + B) / 12.0
    }
}

/// Converts [`YuvRoi`] crops of one video to RGB.
#[derive(Debug, Clone)]
pub struct RgbConverter {
    colour: VideoColour,
    sdr: SdrTables,
    hdr: Option<HdrTables>,
}

impl RgbConverter {
    /// A converter for a video with these colour properties.
    pub fn new(colour: VideoColour) -> Self {
        let hdr = (colour.transfer != ColourTransfer::Sdr)
            .then(|| HdrTables::new(colour.transfer, colour.wide_gamut));
        Self {
            colour,
            sdr: SdrTables::new(colour.matrix, colour.range),
            hdr,
        }
    }

    /// Whether the conversion tone-maps HDR to SDR.
    pub fn is_tone_mapped(&self) -> bool {
        self.hdr.is_some()
    }

    /// Converts `roi` into `out` (resized as needed).
    ///
    /// # Errors
    ///
    /// [`VideoError::Unsupported`] when `roi` holds no copy yet.
    pub fn convert(&self, roi: &YuvRoi, out: &mut RgbImage) -> Result<(), VideoError> {
        let rect = roi
            .rect()
            .ok_or_else(|| VideoError::Unsupported(String::from("empty crop")))?;
        let (w, h) = (rect.width() as usize, rect.height() as usize);
        out.width = rect.width();
        out.height = rect.height();
        out.data.resize(3 * w * h, 0);
        let (luma, (c0, c1)) = (roi.luma(), roi.chroma());
        let cw = w / 2;
        if self.hdr.is_none() && roi.layout() != RoiLayout::P010 {
            self.sdr.convert_8bit(luma, (c0, c1), w, &mut out.data);
            return Ok(());
        }
        // Samples as integers at their native depth: 8 bits, or 10 bits for
        // P010 (stored `<< 6`).
        let sample = |data: &[u8], i: usize| -> i32 {
            match roi.layout() {
                RoiLayout::P010 => {
                    i32::from(u16::from_le_bytes([data[2 * i], data[2 * i + 1]]) >> 6)
                }
                _ => i32::from(data[i]),
            }
        };
        let chroma = |cx: usize, cy: usize| -> (i32, i32) {
            let i = cy * cw + cx;
            match (roi.layout(), c1) {
                (RoiLayout::I420, Some(v)) => (sample(c0, i), sample(v, i)),
                _ => (sample(c0, 2 * i), sample(c0, 2 * i + 1)),
            }
        };
        let ten_bit = roi.layout() == RoiLayout::P010;
        for (row, line) in out.data.chunks_exact_mut(3 * w).enumerate() {
            for (col, px) in line.as_chunks_mut::<3>().0.iter_mut().enumerate() {
                let y = sample(luma, row * w + col);
                let (u, v) = chroma(col / 2, row / 2);
                *px = match &self.hdr {
                    Some(hdr) => hdr.map(self.normalised(y, u, v, ten_bit)),
                    None if ten_bit => {
                        let to8 = |x: i32| ((x + 2) >> 2).clamp(0, 255) as u8;
                        self.sdr.rgb(to8(y), to8(u), to8(v))
                    }
                    None => self.sdr.rgb(y as u8, u as u8, v as u8),
                };
            }
        }
        Ok(())
    }

    /// Non-linear R'G'B' in 0..=1 from code values (HDR path).
    fn normalised(&self, y: i32, u: i32, v: i32, ten_bit: bool) -> [f32; 3] {
        let scale = if ten_bit { 4.0 } else { 1.0 };
        let (y, u, v) = (y as f32 / scale, u as f32 / scale, v as f32 / scale);
        let (yn, un, vn) = match self.colour.range {
            SampleRange::Full => (y / 255.0, (u - 128.0) / 255.0, (v - 128.0) / 255.0),
            _ => ((y - 16.0) / 219.0, (u - 128.0) / 224.0, (v - 128.0) / 224.0),
        };
        let (kr, kb) = weights(self.colour.matrix);
        let (kr, kb) = (kr as f32, kb as f32);
        let kg = 1.0 - kr - kb;
        let r = yn + 2.0 * (1.0 - kr) * vn;
        let b = yn + 2.0 * (1.0 - kb) * un;
        let g = (yn - kr * r - kb * b) / kg;
        [r, g, b].map(|c| c.clamp(0.0, 1.0))
    }
}

#[cfg(test)]
mod tests {
    use blurrd_blur::{SampleRange, YuvMatrix};

    use super::{HdrTables, SdrTables, hlg_inverse_oetf, pq_eotf};
    use crate::colour::ColourTransfer;

    #[test]
    fn sdr_conversion_hits_reference_colours() {
        let c = SdrTables::new(YuvMatrix::Bt709, SampleRange::Limited);
        assert_eq!(c.rgb(16, 128, 128), [0, 0, 0]);
        assert_eq!(c.rgb(235, 128, 128), [255, 255, 255]);
        // FFmpeg truncates: 128 -> 130.41 -> 130, 234 -> 253.84 -> 253.
        assert_eq!(c.rgb(128, 128, 128), [130, 130, 130]);
        assert_eq!(c.rgb(234, 128, 128), [253, 253, 253]);
        // BT.709 limited-range primaries.
        let near = |a: [u8; 3], b: [u8; 3]| a.iter().zip(b).all(|(&x, y)| x.abs_diff(y) <= 2);
        assert!(
            near(c.rgb(63, 102, 240), [255, 0, 0]),
            "{:?}",
            c.rgb(63, 102, 240)
        );
        assert!(
            near(c.rgb(173, 42, 26), [0, 255, 0]),
            "{:?}",
            c.rgb(173, 42, 26)
        );
        assert!(
            near(c.rgb(32, 240, 118), [0, 0, 255]),
            "{:?}",
            c.rgb(32, 240, 118)
        );
        let full = SdrTables::new(YuvMatrix::Bt601, SampleRange::Full);
        assert_eq!(full.rgb(0, 128, 128), [0, 0, 0]);
        assert_eq!(full.rgb(255, 128, 128), [255, 255, 255]);
    }

    #[test]
    fn pixel_pairs_convert_like_single_pixels() {
        let c = SdrTables::new(YuvMatrix::Bt709, SampleRange::Limited);
        let (w, h) = (6, 4);
        let luma: Vec<u8> = (0..w * h).map(|i| (i * 37 % 256) as u8).collect();
        let u: Vec<u8> = (0..w / 2 * h / 2)
            .map(|i| (60 + i * 29 % 140) as u8)
            .collect();
        let v: Vec<u8> = (0..w / 2 * h / 2)
            .map(|i| (200 - i * 23 % 150) as u8)
            .collect();
        let uv: Vec<u8> = u.iter().zip(&v).flat_map(|(&a, &b)| [a, b]).collect();
        let expected: Vec<u8> = (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .flat_map(|(x, y)| {
                let ci = y / 2 * (w / 2) + x / 2;
                c.rgb(luma[y * w + x], u[ci], v[ci])
            })
            .collect();
        let mut planar = vec![0; 3 * w * h];
        c.convert_8bit(&luma, (&u, Some(&v)), w, &mut planar);
        assert_eq!(planar, expected);
        let mut interleaved = vec![0; 3 * w * h];
        c.convert_8bit(&luma, (&uv, None), w, &mut interleaved);
        assert_eq!(interleaved, expected);
    }

    #[test]
    fn transfer_functions_have_known_points() {
        assert!(pq_eotf(0.0) < 1e-3);
        assert!((pq_eotf(1.0) - 10000.0).abs() < 1.0);
        // PQ code for 203 nits (BT.2408 reference white) is about 0.58.
        assert!((pq_eotf(0.5807) - 203.0).abs() < 3.0, "{}", pq_eotf(0.5807));
        assert!((hlg_inverse_oetf(0.5) - 1.0 / 12.0).abs() < 1e-6);
        assert!((hlg_inverse_oetf(1.0) - 1.0).abs() < 1e-3);
    }

    #[test]
    fn tone_mapping_is_monotonic_and_bounded() {
        for transfer in [ColourTransfer::Pq, ColourTransfer::Hlg] {
            let t = HdrTables::new(transfer, true);
            assert_eq!(t.map([0.0; 3]), [0, 0, 0]);
            let mut last = 0;
            for i in 0..=100 {
                let s = i as f32 / 100.0;
                let [r, g, b] = t.map([s; 3]);
                assert!(
                    r.abs_diff(g) <= 1 && g.abs_diff(b) <= 1,
                    "{transfer:?}: grey stays grey"
                );
                assert!(r >= last, "{transfer:?}: monotonic at {s}");
                last = r;
            }
            assert!(last >= 250, "{transfer:?}: peak maps near white ({last})");
        }
        // Reference white lands in the upper middle, not clipped.
        let pq = HdrTables::new(ColourTransfer::Pq, true);
        let [w, ..] = pq.map([0.5807; 3]);
        assert!((170..=215).contains(&w), "reference white -> {w}");
    }
}
