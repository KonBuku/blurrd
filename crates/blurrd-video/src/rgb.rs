//! Converting YUV crops to 8-bit RGB for the detectors and OCR.
//!
//! SDR video is converted exactly like the reference the detector
//! thresholds were tuned on, FFmpeg's default `-pix_fmt bgr24` conversion:
//! the tagged matrix and range to full-range RGB, each chroma sample used
//! for its 2×2 luma block, with swscale's truncation (bit-identical on real
//! recordings). 10-bit SDR samples are rounded to 8 bits first.
//!
//! HDR video (PQ or HLG) is tone-mapped to SDR BT.709 before the detectors
//! classify it (plan 7.3): linear light from the transfer function, a
//! luminance-preserving curve that is linear up to the HUD's paper white
//! (mapped to the SDR HUD white) and rolls the highlights off above it, a
//! BT.2020 to BT.709 gamut conversion with clipping, and the BT.1886
//! (gamma 2.4) encoding. The paper white is the luminance at which the game
//! draws its white HUD text: the BT.2408 reference white (203 nits) unless
//! measured from the video's HUD crops ([`PaperWhite`]). A measurement that
//! is not confident leaves the default; the engine then reads the video
//! with Safe geometry and warns (`HdrToneMapped`).

#![allow(
    clippy::many_single_char_names,
    reason = "colour maths uses the conventional names y, u, v, r, g, b"
)]

use blurrd_blur::{SampleRange, YuvMatrix};

use crate::VideoError;
use crate::colour::{ColourTransfer, VideoColour};
use crate::roi::{RoiLayout, YuvRoi};

/// SDR reference white in nits (ITU-R BT.2408): the paper white unless
/// measured.
pub const REFERENCE_WHITE_NITS: f32 = 203.0;
/// Display peak in nits assumed by HLG's system gamma.
const PEAK_NITS: f32 = 1000.0;
/// Entries of the transfer function look-up tables.
const LUT_SIZE: usize = 4096;
/// Linear SDR light the paper white maps to: BT.1886 encodes 0.82 as code
/// 235, the white of HUD text in SDR recordings.
const PAPER_WHITE_SDR: f32 = 0.82;
/// Highlights above the paper white roll off into the remaining headroom:
/// twice the paper white takes this share of it.
const ROLL_OFF: f32 = 0.5;
/// Bins of the paper-white histogram, logarithmic from
/// [`PAPER_WHITE_MIN_NITS`] to [`PAPER_WHITE_MAX_NITS`].
const PAPER_WHITE_BINS: usize = 256;
/// A plausible paper white lies between these luminances in nits (the HDR
/// settings of games and of the desktop range from about 80 to 1,000).
const PAPER_WHITE_MIN_NITS: f32 = 40.0;
/// See [`PAPER_WHITE_MIN_NITS`].
const PAPER_WHITE_MAX_NITS: f32 = 4000.0;
/// The paper white is measured among the brightest pixels of the HUD
/// crops: this share of them (darker uniform panels, such as the chat box,
/// lie below).
const BRIGHT_SHARE: f64 = 0.25;
/// White text is drawn at one luminance, game highlights spread over
/// many: a peak of the bright pixels is the paper white if at least this
/// many pixels have its luminance ...
const MIN_WHITE_PIXELS: u32 = 400;
/// ... and at least this share of the pixels at or above it lies within
/// one bin of it ...
const MIN_WHITE_SHARE: f64 = 0.25;
/// ... and those bins hold at least this many times as many pixels per bin
/// as the bins a few steps away on both sides ([`PEAK_FLANK`]): a peak,
/// not the dense end of a smooth spread of game light.
const MIN_PEAKEDNESS: f64 = 4.0;
/// The flanks of the paper-white peak: bins this many steps away.
const PEAK_FLANK: (usize, usize) = (3, 8);
/// A pixel counts for the paper white only if its channels differ by at
/// most this share of the largest in linear light: white text, not a team
/// colour's bar.
const NEUTRAL_SHARE: f32 = 0.1;

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
    /// Whether the samples are limited range (16..=235 luma).
    limited: bool,
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
            limited: range != SampleRange::Full,
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
    /// The 4 × 4 block means (each channel `(sum + 8) / 16`) of the RGB an
    /// 8-bit crop `w × h` (even) converts to, laid out like
    /// [`SdrTables::convert_8bit`]'s input, into `out` (`w / 4 × h / 4`
    /// pixels; right and bottom remainders narrower than 4 are left out).
    ///
    /// Byte for byte the means of [`SdrTables::convert_8bit`]'s output,
    /// without writing the converted pixels: the clamped values are summed
    /// in registers, column by column.
    fn means_4x4(
        &self,
        luma: &[u8],
        (c0, c1): (&[u8], Option<&[u8]>),
        (w, h): (usize, usize),
        out: &mut [u8],
    ) {
        let (cw, qw) = (w / 2, w / 4);
        // Per chroma row: the luma terms of its two rows and each channel's
        // chroma offsets (one per column, repeated for both columns of a
        // block); the clamped values are summed per column over the four
        // rows of an output row, then over four columns. The per-column
        // loops run over equal-length slices, which the compiler vectorises.
        let mut terms = [vec![0i16; w], vec![0i16; w]];
        let mut offsets = [vec![0i16; cw], vec![0i16; cw], vec![0i16; cw]];
        let mut repeated = vec![0i16; w];
        // Per channel and column, the sum over four rows (at most 4 × 255).
        let mut columns = [vec![0i16; w], vec![0i16; w], vec![0i16; w]];
        for (qy, line) in out.chunks_exact_mut(3 * qw).take(h / 4).enumerate() {
            for channel in &mut columns {
                channel.fill(0);
            }
            for cy in 2 * qy..2 * qy + 2 {
                for (row, term) in terms.iter_mut().enumerate() {
                    let codes = &luma[(2 * cy + row) * w..][..w];
                    // One loop per range, each a plain vectorisable map.
                    if self.limited {
                        for (t, &code) in term.iter_mut().zip(codes) {
                            *t = limited_luma_term(code);
                        }
                    } else {
                        for (t, &code) in term.iter_mut().zip(codes) {
                            *t = i16::from(code);
                        }
                    }
                }
                self.chroma_offsets(c0, c1, cy, cw, &mut offsets);
                for (offset, column) in offsets.iter().zip(&mut columns) {
                    for (pair, &o) in repeated.as_chunks_mut::<2>().0.iter_mut().zip(offset) {
                        *pair = [o; 2];
                    }
                    for (((sum, &top), &bottom), &o) in column
                        .iter_mut()
                        .zip(&terms[0])
                        .zip(&terms[1])
                        .zip(&repeated)
                    {
                        *sum += (top + o).clamp(0, 255) + (bottom + o).clamp(0, 255);
                    }
                }
            }
            let [r, g, b] = &columns;
            let quads = r
                .as_chunks::<4>()
                .0
                .iter()
                .zip(g.as_chunks::<4>().0)
                .zip(b.as_chunks::<4>().0);
            for (px, ((r, g), b)) in line.as_chunks_mut::<3>().0.iter_mut().zip(quads) {
                *px = [r, g, b].map(|q| ((q[0] + q[1] + q[2] + q[3] + 8) / 16) as u8);
            }
        }
    }

    /// The red, green and blue chroma offsets of chroma row `cy` (`cw`
    /// samples) into `offsets`.
    fn chroma_offsets(
        &self,
        c0: &[u8],
        c1: Option<&[u8]>,
        cy: usize,
        cw: usize,
        [r, g, b]: &mut [Vec<i16>; 3],
    ) {
        let mut set = |i: usize, u: u8, v: u8| {
            let (u, v) = (usize::from(u), usize::from(v));
            r[i] = self.r_v[v];
            g[i] = self.g_u[u] + self.g_v[v];
            b[i] = self.b_u[u];
        };
        if let Some(v) = c1 {
            let (us, vs) = (&c0[cy * cw..][..cw], &v[cy * cw..][..cw]);
            for (i, (&u, &v)) in us.iter().zip(vs).enumerate() {
                set(i, u, v);
            }
        } else {
            let uvs = c0[2 * cy * cw..][..2 * cw].as_chunks::<2>().0;
            for (i, &[u, v]) in uvs.iter().enumerate() {
                set(i, u, v);
            }
        }
    }

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

/// The limited-range luma term of `code`, `(code - 16) × 255 / 219`
/// rounded down, in arithmetic the compiler can vectorise. The fixed-point
/// form gives every entry of [`SdrTables`]'s limited-range luma table,
/// including those where its floating point lands just below a whole
/// number (checked against the table in the tests).
#[inline]
fn limited_luma_term(code: u8) -> i16 {
    (((i32::from(code) - 16) * 76_305 + 885) >> 16) as i16
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
    /// Luminance in nits that maps to the SDR HUD white.
    paper_white: f32,
}

impl HdrTables {
    fn new(transfer: ColourTransfer, wide_gamut: bool, paper_white: f32) -> Self {
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
            paper_white,
        }
    }

    #[inline]
    fn lookup(table: &[f32], x: f32) -> f32 {
        table[(x.clamp(0.0, 1.0) * (LUT_SIZE - 1) as f32).round() as usize]
    }

    /// Display light in nits of normalised non-linear BT.2020 R'G'B'.
    fn linear(&self, rgb: [f32; 3]) -> [f32; 3] {
        let lin = rgb.map(|c| Self::lookup(&self.eotf, c));
        if !self.hlg {
            return lin;
        }
        // HLG: scene light (0..=1) to display light with the BT.2100 OOTF
        // for a 1000-nit display (system gamma 1.2).
        let gain = PEAK_NITS * luminance(lin).max(1e-6).powf(0.2);
        lin.map(|c| c * gain)
    }

    /// Normalised non-linear BT.2020 R'G'B' to 8-bit SDR RGB.
    fn map(&self, rgb: [f32; 3]) -> [u8; 3] {
        // Luminance-preserving tone curve: linear up to the paper white,
        // which lands on the SDR HUD white, then a roll-off into the
        // remaining headroom.
        let rel = self.linear(rgb).map(|c| c / self.paper_white);
        let l = luminance(rel);
        let mapped = if l <= 1.0 {
            PAPER_WHITE_SDR * l
        } else {
            let over = l - 1.0;
            PAPER_WHITE_SDR + (1.0 - PAPER_WHITE_SDR) * over / (over + ROLL_OFF.recip() - 1.0)
        };
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

/// BT.2020 luminance of linear RGB.
fn luminance([r, g, b]: [f32; 3]) -> f32 {
    0.2627 * r + 0.6780 * g + 0.0593 * b
}

/// The paper white of an HDR video, measured from its HUD crops: the
/// luminance at which the game draws its white text.
///
/// White HUD text is drawn at one luminance in every frame, while game
/// highlights spread over many: among the brightest quarter of the crops'
/// neutral pixels, the fullest sharp peak is the paper white, if at least
/// 400 pixels gather there, a quarter of the pixels at or above it, and
/// four times as many per luminance step as a few steps away.
#[derive(Debug, Clone)]
pub struct PaperWhite {
    histogram: Vec<u32>,
}

impl Default for PaperWhite {
    fn default() -> Self {
        Self {
            histogram: vec![0; PAPER_WHITE_BINS],
        }
    }
}

impl PaperWhite {
    /// An empty measurement.
    pub fn new() -> Self {
        Self::default()
    }

    /// The span of the histogram in natural-log units.
    fn span() -> f32 {
        (PAPER_WHITE_MAX_NITS / PAPER_WHITE_MIN_NITS).ln()
    }

    /// Adds a pixel of display luminance `nits`; luminances outside the
    /// plausible range do not count.
    pub fn add(&mut self, nits: f32) {
        if (PAPER_WHITE_MIN_NITS..PAPER_WHITE_MAX_NITS).contains(&nits) {
            let t = (nits / PAPER_WHITE_MIN_NITS).ln() / Self::span();
            let bin = ((t * PAPER_WHITE_BINS as f32) as usize).min(PAPER_WHITE_BINS - 1);
            self.histogram[bin] += 1;
        }
    }

    /// The paper white in nits, or `None` if the measurement is not
    /// confident.
    pub fn estimate(&self) -> Option<f32> {
        let hist = &self.histogram;
        let count = |range: std::ops::Range<usize>| -> u64 {
            hist[range.start..range.end.min(PAPER_WHITE_BINS)]
                .iter()
                .map(|&n| u64::from(n))
                .sum()
        };
        let total = count(0..PAPER_WHITE_BINS);
        let wanted = (total as f64 * BRIGHT_SHARE).ceil() as u64;
        // The lowest bin of the brightest share.
        let (mut from, mut seen) = (PAPER_WHITE_BINS, 0u64);
        while from > 0 && seen < wanted {
            from -= 1;
            seen += u64::from(hist[from]);
        }
        // The fullest sharp peak there: a local maximum with enough pixels,
        // well above its flanks, and a fair share of everything at or
        // above it (highlights spread over many luminances).
        let peak = |bin: usize| -> Option<u64> {
            let around = count(bin.saturating_sub(1)..bin + 2);
            let local_max = (bin.saturating_sub(1)..(bin + 2).min(PAPER_WHITE_BINS))
                .all(|b| hist[b] <= hist[bin]);
            let flank: Vec<u32> = (PEAK_FLANK.0..=PEAK_FLANK.1)
                .flat_map(|d| [bin.checked_sub(d), Some(bin + d)])
                .flatten()
                .filter_map(|b| hist.get(b).copied())
                .collect();
            let flank_mean =
                flank.iter().map(|&n| f64::from(n)).sum::<f64>() / flank.len().max(1) as f64;
            let peaked = around as f64 / 3.0 >= MIN_PEAKEDNESS * flank_mean;
            let gathered = around as f64
                >= MIN_WHITE_SHARE * count(bin.saturating_sub(1)..PAPER_WHITE_BINS) as f64;
            (hist[bin] >= MIN_WHITE_PIXELS && local_max && peaked && gathered).then_some(around)
        };
        let (bin, _) = (from..PAPER_WHITE_BINS)
            .filter_map(|b| peak(b).map(|n| (b, n)))
            .max_by_key(|&(_, n)| n)?;
        let centre = bin as f32 + 0.5;
        Some(PAPER_WHITE_MIN_NITS * (centre / PAPER_WHITE_BINS as f32 * Self::span()).exp())
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
    /// A converter for a video with these colour properties; HDR video is
    /// tone-mapped with the reference paper white
    /// ([`REFERENCE_WHITE_NITS`]).
    pub fn new(colour: VideoColour) -> Self {
        Self::with_paper_white(colour, REFERENCE_WHITE_NITS)
    }

    /// A converter that tone-maps HDR video with the paper white
    /// `paper_white` in nits (measured with [`PaperWhite`]); SDR video is
    /// converted as by [`RgbConverter::new`].
    pub fn with_paper_white(colour: VideoColour, paper_white: f32) -> Self {
        let hdr = (colour.transfer != ColourTransfer::Sdr)
            .then(|| HdrTables::new(colour.transfer, colour.wide_gamut, paper_white));
        Self {
            colour,
            sdr: SdrTables::new(colour.matrix, colour.range),
            hdr,
        }
    }

    /// Adds the display luminance of the neutral pixels of `roi` to `into`,
    /// for HDR video (SDR video adds nothing). HUD text is white; the
    /// coloured bars behind it are uniform too and would otherwise pass for
    /// the paper white (a pixel is neutral if its channels differ by at most
    /// a tenth of the largest).
    ///
    /// # Errors
    ///
    /// [`VideoError::Unsupported`] when `roi` holds no copy yet.
    pub fn measure_paper_white(
        &self,
        roi: &YuvRoi,
        into: &mut PaperWhite,
    ) -> Result<(), VideoError> {
        let Some(hdr) = &self.hdr else {
            return Ok(());
        };
        let rect = roi
            .rect()
            .ok_or_else(|| VideoError::Unsupported(String::from("empty crop")))?;
        let (w, h) = (rect.width() as usize, rect.height() as usize);
        let (luma, (c0, c1)) = (roi.luma(), roi.chroma());
        let cw = w / 2;
        let ten_bit = roi.layout() == RoiLayout::P010;
        let sample = |data: &[u8], i: usize| -> i32 {
            if ten_bit {
                i32::from(u16::from_le_bytes([data[2 * i], data[2 * i + 1]]) >> 6)
            } else {
                i32::from(data[i])
            }
        };
        for row in 0..h {
            for col in 0..w {
                let i = (row / 2) * cw + col / 2;
                let (u, v) = match (roi.layout(), c1) {
                    (RoiLayout::I420, Some(v)) => (sample(c0, i), sample(v, i)),
                    _ => (sample(c0, 2 * i), sample(c0, 2 * i + 1)),
                };
                let y = sample(luma, row * w + col);
                let light = hdr.linear(self.normalised(y, u, v, ten_bit));
                let (max, min) = (
                    light[0].max(light[1]).max(light[2]),
                    light[0].min(light[1]).min(light[2]),
                );
                if max - min <= NEUTRAL_SHARE * max {
                    into.add(luminance(light));
                }
            }
        }
        Ok(())
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

    /// The 4 × 4 block means of what [`RgbConverter::convert`] makes of
    /// `roi` (each channel `(sum + 8) / 16`; right and bottom remainders
    /// narrower than 4 pixels left out), into `out`: a downscaled view
    /// without converting every pixel. Only 8-bit SDR crops take this
    /// path; for 10-bit or HDR crops it returns `false` and leaves `out`
    /// alone, and the caller converts them in full.
    ///
    /// # Errors
    ///
    /// [`VideoError::Unsupported`] when `roi` holds no copy yet.
    pub fn convert_means_4x4(&self, roi: &YuvRoi, out: &mut RgbImage) -> Result<bool, VideoError> {
        let rect = roi
            .rect()
            .ok_or_else(|| VideoError::Unsupported(String::from("empty crop")))?;
        if self.hdr.is_some() || roi.layout() == RoiLayout::P010 {
            return Ok(false);
        }
        let (w, h) = (rect.width() as usize, rect.height() as usize);
        out.width = (w / 4) as u32;
        out.height = (h / 4) as u32;
        out.data.clear();
        out.data.resize(3 * (w / 4) * (h / 4), 0);
        self.sdr
            .means_4x4(roi.luma(), roi.chroma(), (w, h), &mut out.data);
        Ok(true)
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

    use blurrd_core::{ColorInfo, ColorRange, RectI};

    use super::{
        HdrTables, PAPER_WHITE_SDR, PaperWhite, RgbConverter, RgbImage, SdrTables,
        hlg_inverse_oetf, limited_luma_term, pq_eotf,
    };
    use crate::colour::{ColourTransfer, VideoColour};
    use crate::roi::{RoiLayout, YuvRoi};

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
    fn luma_terms_match_the_table() {
        let limited = SdrTables::new(YuvMatrix::Bt709, SampleRange::Limited);
        let full = SdrTables::new(YuvMatrix::Bt709, SampleRange::Full);
        assert!(limited.limited && !full.limited);
        for code in 0..=255u8 {
            let i = usize::from(code);
            assert_eq!(limited_luma_term(code), limited.y[i], "{code}");
            assert_eq!(i16::from(code), full.y[i], "{code}");
        }
    }

    #[test]
    fn block_means_equal_the_means_of_the_converted_pixels() {
        // Full-scale noise clamps often (black and white crushes, saturated
        // chroma); every matrix and range, planar and interleaved chroma,
        // and sizes with remainders that are left out.
        let (w, h) = (22, 18);
        let mut seed = 0x2545_f491_u32;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed >> 24) as u8
        };
        let luma: Vec<u8> = (0..w * h).map(|_| next()).collect();
        let u: Vec<u8> = (0..w / 2 * h / 2).map(|_| next()).collect();
        let v: Vec<u8> = (0..w / 2 * h / 2).map(|_| next()).collect();
        let uv: Vec<u8> = u.iter().zip(&v).flat_map(|(&a, &b)| [a, b]).collect();
        for matrix in [YuvMatrix::Bt601, YuvMatrix::Bt709, YuvMatrix::Bt2020] {
            for range in [SampleRange::Limited, SampleRange::Full] {
                let c = SdrTables::new(matrix, range);
                let mut rgb = vec![0u8; 3 * w * h];
                c.convert_8bit(&luma, (&u, Some(&v)), w, &mut rgb);
                let (qw, qh) = (w / 4, h / 4);
                let expected: Vec<u8> = (0..qh * qw * 3)
                    .map(|i| {
                        let (qy, qx, ch) = (i / 3 / qw, i / 3 % qw, i % 3);
                        let sum: u32 = (0..16)
                            .map(|k| {
                                u32::from(rgb[3 * ((4 * qy + k / 4) * w + 4 * qx + k % 4) + ch])
                            })
                            .sum();
                        ((sum + 8) / 16) as u8
                    })
                    .collect();
                for chroma in [(&u[..], Some(&v[..])), (&uv[..], None)] {
                    let mut means = vec![0u8; 3 * qw * qh];
                    c.means_4x4(&luma, chroma, (w, h), &mut means);
                    assert_eq!(means, expected, "{matrix:?} {range:?}");
                }
            }
        }
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
            let t = HdrTables::new(transfer, true, super::REFERENCE_WHITE_NITS);
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
        // The paper white lands on the SDR HUD white, below clipping.
        let pq = HdrTables::new(ColourTransfer::Pq, true, super::REFERENCE_WHITE_NITS);
        let [w, ..] = pq.map([0.5807; 3]);
        assert!((230..=240).contains(&w), "reference white -> {w}");
    }

    /// The PQ signal of `nits`.
    fn pq_signal(nits: f32) -> f32 {
        const M1: f32 = 2610.0 / 16384.0;
        const M2: f32 = 2523.0 / 4096.0 * 128.0;
        const C1: f32 = 3424.0 / 4096.0;
        const C2: f32 = 2413.0 / 4096.0 * 32.0;
        const C3: f32 = 2392.0 / 4096.0 * 32.0;
        let y = (nits / 10000.0).powf(M1);
        ((C1 + C2 * y) / (1.0 + C3 * y)).powf(M2)
    }

    #[test]
    fn measures_the_paper_white_of_hud_text() {
        // A HUD crop of an HDR video with its white text at 400 nits: the
        // game behind it in dim to bright greys with a few highlights
        // brighter than the text.
        let mut pw = PaperWhite::new();
        for i in 0..200_000u32 {
            pw.add(5.0 + (i % 997) as f32 * 0.3);
        }
        for _ in 0..3_000 {
            pw.add(400.0);
        }
        for i in 0..2_000u32 {
            pw.add(450.0 + (i % 400) as f32 * 4.0);
        }
        let white = pw.estimate().unwrap();
        assert!((white - 400.0).abs() < 400.0 * 0.03, "{white}");
        // Text and game mapped with the measured paper white: the text on
        // the SDR HUD white, the game below it.
        let t = HdrTables::new(ColourTransfer::Pq, true, white);
        let [text, ..] = t.map([pq_signal(400.0); 3]);
        let [game, ..] = t.map([pq_signal(100.0); 3]);
        assert!((230..=240).contains(&text), "text -> {text}");
        assert!(game < 160, "game -> {game}");
    }

    /// Linear BT.2020 display light in nits of the SDR BT.709 colour
    /// `sdr` in a video whose paper white is `paper_white`: the inverse of
    /// the tone curve below the paper white.
    fn hdr_light(sdr: [u8; 3], paper_white: f32) -> [f32; 3] {
        let [r, g, b] =
            sdr.map(|c| (f32::from(c) / 255.0).powf(2.4) / PAPER_WHITE_SDR * paper_white);
        [
            0.627_4 * r + 0.329_3 * g + 0.043_3 * b,
            0.069_1 * r + 0.919_5 * g + 0.011_4 * b,
            0.016_4 * r + 0.088_0 * g + 0.895_6 * b,
        ]
    }

    /// A P010 BT.2020 PQ crop of `w × h` pixels whose 2 × 2 blocks have the
    /// linear display light (nits) `light(bx, by)`.
    fn pq_crop(w: usize, h: usize, light: impl Fn(usize, usize) -> [f32; 3]) -> YuvRoi {
        let code = |v: f32| ((v.round().clamp(0.0, 1023.0) as u16) << 6).to_le_bytes();
        let (mut y, mut uv) = (vec![0u8; 2 * w * h], vec![0u8; w * h]);
        for by in 0..h / 2 {
            for bx in 0..w / 2 {
                let [r, g, b] = light(bx, by).map(pq_signal);
                let luma = 0.2627 * r + 0.6780 * g + 0.0593 * b;
                let (cb, cr) = ((b - luma) / 1.8814, (r - luma) / 1.4746);
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let i = (2 * by + dy) * w + 2 * bx + dx;
                    y[2 * i..2 * i + 2].copy_from_slice(&code(64.0 + 876.0 * luma));
                }
                let i = by * (w / 2) + bx;
                uv[4 * i..4 * i + 2].copy_from_slice(&code(512.0 + 896.0 * cb));
                uv[4 * i + 2..4 * i + 4].copy_from_slice(&code(512.0 + 896.0 * cr));
            }
        }
        let size = |n: usize| i32::try_from(n).unwrap();
        let rect = RectI::new(0, 0, size(w), size(h)).unwrap();
        YuvRoi::from_parts(rect, RoiLayout::P010, y, uv, Vec::new())
    }

    /// A kill-feed-like HDR crop: game light up to half the paper white
    /// with a few highlights at three times it above, a teal and a red bar
    /// below, white text strokes on the bars,
    /// with the HUD's paper white at `paper_white` nits (or no text).
    fn hud_crop(paper_white: f32, text: bool) -> YuvRoi {
        const TEAL: [u8; 3] = [100, 186, 162];
        const RED: [u8; 3] = [230, 98, 93];
        pq_crop(256, 96, |bx, by| match by {
            0..8 if bx % 23 == 0 => [3.0 * paper_white; 3],
            0..8 => [2.0 + paper_white * (bx + 7 * by) as f32 / 400.0; 3],
            _ if text && (12..20).contains(&(by % 24)) && bx % 4 == 1 => [paper_white; 3],
            8..28 => hdr_light(TEAL, paper_white),
            _ => hdr_light(RED, paper_white),
        })
    }

    /// The SDR pixel `(x, y)` of `img`.
    fn px(img: &RgbImage, x: usize, y: usize) -> [u8; 3] {
        let i = 3 * (y * img.width as usize + x);
        [img.data[i], img.data[i + 1], img.data[i + 2]]
    }

    #[test]
    fn hdr_hud_crops_are_mapped_to_the_sdr_colours_the_detectors_know() {
        let info = ColorInfo {
            primaries: "bt2020".to_owned(),
            transfer: "smpte2084".to_owned(),
            matrix: "bt2020nc".to_owned(),
            range: ColorRange::Limited,
        };
        let colour = VideoColour::from_info(&info, 2160);
        let near =
            |a: [u8; 3], b: [u8; 3], tol: u8| a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= tol);
        for paper_white in [120.0, 203.0, 400.0, 800.0] {
            let crop = hud_crop(paper_white, true);
            let mut measured = PaperWhite::new();
            RgbConverter::new(colour)
                .measure_paper_white(&crop, &mut measured)
                .unwrap();
            let white = measured.estimate().unwrap();
            assert!(
                (white / paper_white - 1.0).abs() < 0.03,
                "{paper_white}: {white}"
            );
            let converter = RgbConverter::with_paper_white(colour, white);
            assert!(converter.is_tone_mapped());
            let mut img = RgbImage::default();
            converter.convert(&crop, &mut img).unwrap();
            // Text on the SDR HUD white, bars in their SDR colours, as the
            // pixel classes expect them.
            let (text, teal, red) = (px(&img, 2, 26), px(&img, 6, 26), px(&img, 6, 70));
            assert!(near(text, [235; 3], 4), "{paper_white}: text {text:?}");
            assert!(
                near(teal, [100, 186, 162], 6),
                "{paper_white}: teal {teal:?}"
            );
            assert!(near(red, [230, 98, 93], 6), "{paper_white}: red {red:?}");
            if paper_white > 300.0 {
                // The reference white alone would wash the bars out.
                let mut reference = RgbImage::default();
                RgbConverter::new(colour)
                    .convert(&crop, &mut reference)
                    .unwrap();
                let teal = px(&reference, 6, 26);
                assert!(!near(teal, [100, 186, 162], 20), "{paper_white}: {teal:?}");
            }
        }
        // Without HUD text the paper white is not confident: the engine
        // keeps the reference white, reads the video in Safe mode and
        // warns (HdrToneMapped).
        let mut measured = PaperWhite::new();
        RgbConverter::new(colour)
            .measure_paper_white(&hud_crop(400.0, false), &mut measured)
            .unwrap();
        assert_eq!(measured.estimate(), None);
    }

    #[test]
    fn a_frame_without_white_text_is_not_confident() {
        // Game light spread over every luminance: no HUD white.
        let mut pw = PaperWhite::new();
        for i in 0..200_000u32 {
            pw.add(1.0 + (i % 9_973) as f32 * 0.2);
        }
        assert_eq!(pw.estimate(), None);
        assert_eq!(PaperWhite::new().estimate(), None);
    }
}
