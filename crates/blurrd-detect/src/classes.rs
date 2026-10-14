//! Pixel classes of the kill feed and banner detectors (port spec
//! section 2).
//!
//! Every pixel of an RGB crop is classified once into bit flags:
//!
//! | Flag              | Spec  | Rule (8-bit OpenCV HSV, `mn`/`mx` = min/max of R, G, B)  |
//! | ----------------- | ----- | --------------------------------------------------------- |
//! | [`Class::Teal`]   | `G`   | `66 ≤ H ≤ 96`, `S ≥ 45`, `V ≥ 70`                          |
//! | [`Class::Red`]    | `R`   | `H ≤ 10` or `H ≥ 170`, `S ≥ 85`, `V ≥ 110`                 |
//! | [`Class::White`]  | `W`   | `mn ≥ 170`, `mx − mn ≤ 70`                                 |
//! | [`Class::PureWhite`] | `WS` | `mn ≥ 215`, `mx − mn ≤ 45`                              |
//! | [`Class::Text`]   | `T`   | top-hat of `mn` (7 × 7) `≥ 35`, `mn ≥ 165`, `mx − mn ≤ 75` |
//! | [`Class::Bar`]    | `BAR` | `G` with `V ≥ 120` and `S ≥ 60`, or `R` with `V ≥ 150`     |
//!
//! The thresholds were tuned on FFmpeg's `bgr24` conversion of BT.709
//! recordings, which `blurrd-video` reproduces bit for bit. HSV follows
//! OpenCV's 8-bit `COLOR_BGR2HSV` exactly (H in `0..180`), including its
//! fixed-point rounding; a parity table generated with OpenCV checks it.
//! The hue ranges are only a starting point: colour-blind modes change the
//! bar colours, so later milestones learn them per video
//! ([`ClassParams`]).

#![allow(
    clippy::many_single_char_names,
    reason = "colour maths uses the conventional names r, g, b, h, s, v"
)]

use blurrd_core::RectI;

use crate::image::{BitMask, GrayImage, RgbView};
use crate::morph::{MorphScratch, white_tophat};

/// Fixed-point shift of OpenCV's HSV conversion.
const HSV_SHIFT: u32 = 12;

/// `round((255 << 12) / i)`, OpenCV's saturation divisor table.
const SDIV: [i32; 256] = div_table(255 << HSV_SHIFT);

/// `round((180 << 12) / (6 i))`, OpenCV's hue divisor table for H in
/// `0..180`.
const HDIV: [i32; 256] = div_table((180 << HSV_SHIFT) / 6);

/// `round(n / i)` for `i` in `1..256` (0 for `i = 0`). For both tables the
/// quotient is never exactly halfway between two integers, so this equals
/// OpenCV's `saturate_cast<int>` of the floating-point quotient.
const fn div_table(n: i32) -> [i32; 256] {
    let mut table = [0; 256];
    let mut i = 1;
    while i < 256 {
        table[i] = (2 * n + i as i32) / (2 * i as i32);
        i += 1;
    }
    table
}

/// OpenCV's 8-bit HSV of an RGB colour: `[H, S, V]` with H in `0..180`.
///
/// Bit-exact with `cv2.cvtColor(bgr, cv2.COLOR_BGR2HSV)` for every input.
pub fn hsv(rgb: [u8; 3]) -> [u8; 3] {
    let v = rgb[0].max(rgb[1]).max(rgb[2]);
    let diff = v - rgb[0].min(rgb[1]).min(rgb[2]);
    [hue(rgb, v, diff), saturation(v, diff), v]
}

/// OpenCV's 8-bit saturation from `V = max(R, G, B)` and the chroma.
#[inline]
fn saturation(v: u8, diff: u8) -> u8 {
    ((i32::from(diff) * SDIV[usize::from(v)] + (1 << (HSV_SHIFT - 1))) >> HSV_SHIFT) as u8
}

/// OpenCV's 8-bit hue (`0..180`), branch-free like OpenCV's own scalar
/// code: red has priority over green when both are the maximum.
#[inline]
fn hue(rgb: [u8; 3], v: u8, diff: u8) -> u8 {
    let vr = -i32::from(v == rgb[0]);
    let vg = -i32::from(v == rgb[1]);
    let [r, g, b] = rgb.map(i32::from);
    let d = i32::from(diff);
    let num = (vr & (g - b)) + (!vr & ((vg & (b - r + 2 * d)) + (!vg & (r - g + 4 * d))));
    let h = (num * HDIV[usize::from(diff)] + (1 << (HSV_SHIFT - 1))) >> HSV_SHIFT;
    // Add 180 to negative hues.
    (h + 180 * i32::from(h < 0)) as u8
}

/// One pixel class, stored as a bit flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Class {
    /// Teal (ally) kill-feed bar colour, spec `G`.
    Teal = 1,
    /// Red (enemy) kill-feed bar colour, spec `R`.
    Red = 2,
    /// White-ish pixels, spec `W`.
    White = 4,
    /// Pure white pixels (weapon icons), spec `WS`.
    PureWhite = 8,
    /// Thin bright text strokes, spec `T`.
    Text = 16,
    /// Opaque kill-feed bar, spec `BAR`.
    Bar = 32,
}

impl Class {
    /// The flag bit of the class.
    pub const fn bit(self) -> u8 {
        self as u8
    }

    /// The flag bit if `on`, else 0 (without a branch).
    const fn bit_if(self, on: bool) -> u8 {
        self.bit() * on as u8
    }
}

/// The thresholds of the pixel classes. [`Default`] gives the constants of
/// the port spec; the hue ranges may be replaced by learned ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassParams {
    /// Inclusive hue range (`0..180`) of [`Class::Teal`]. A range whose
    /// start is above its end wraps around 0 ([`in_hue_range`]).
    pub teal_hue: (u8, u8),
    /// Minimum saturation of [`Class::Teal`].
    pub teal_min_s: u8,
    /// Minimum value of [`Class::Teal`].
    pub teal_min_v: u8,
    /// [`Class::Red`] hues lie in the circular range `red_hue_min ..=
    /// red_hue_max`: the default (170 to 10) wraps around 0, so it means
    /// `≥ red_hue_min` or `≤ red_hue_max`; a learned range may not wrap
    /// ([`in_hue_range`]).
    pub red_hue_max: u8,
    /// See [`ClassParams::red_hue_max`].
    pub red_hue_min: u8,
    /// Minimum saturation of [`Class::Red`].
    pub red_min_s: u8,
    /// Minimum value of [`Class::Red`].
    pub red_min_v: u8,
    /// Minimum `mn` of [`Class::White`].
    pub white_min: u8,
    /// Maximum chroma of [`Class::White`].
    pub white_max_chroma: u8,
    /// Minimum `mn` of [`Class::PureWhite`].
    pub pure_white_min: u8,
    /// Maximum chroma of [`Class::PureWhite`].
    pub pure_white_max_chroma: u8,
    /// Side of the square top-hat kernel of [`Class::Text`].
    pub tophat_size: u32,
    /// Minimum top-hat response of [`Class::Text`].
    pub text_min_tophat: u8,
    /// Minimum `mn` of [`Class::Text`].
    pub text_min_mn: u8,
    /// Maximum chroma of [`Class::Text`].
    pub text_max_chroma: u8,
    /// Minimum value of a teal [`Class::Bar`] pixel.
    pub bar_teal_min_v: u8,
    /// Minimum saturation of a teal [`Class::Bar`] pixel.
    pub bar_teal_min_s: u8,
    /// Minimum value of a red [`Class::Bar`] pixel.
    pub bar_red_min_v: u8,
}

impl Default for ClassParams {
    /// The constants of the port spec, section 2.
    fn default() -> Self {
        Self {
            teal_hue: (66, 96),
            teal_min_s: 45,
            teal_min_v: 70,
            red_hue_max: 10,
            red_hue_min: 170,
            red_min_s: 85,
            red_min_v: 110,
            white_min: 170,
            white_max_chroma: 70,
            pure_white_min: 215,
            pure_white_max_chroma: 45,
            tophat_size: 7,
            text_min_tophat: 35,
            text_min_mn: 165,
            text_max_chroma: 75,
            bar_teal_min_v: 120,
            bar_teal_min_s: 60,
            bar_red_min_v: 150,
        }
    }
}

impl ClassParams {
    /// The white level of HUD text the brightness thresholds were tuned
    /// for: the brightest strokes of kill-feed names reach 229 to 249 in
    /// unfiltered recordings (measured on the parity excerpts).
    pub const REFERENCE_WHITE: u8 = 225;

    /// The thresholds shift down by at most this much.
    const MAX_WHITE_SHIFT: u8 = 60;

    /// The thresholds for a frame whose white HUD text only reaches
    /// `white`, for example under a darkening brightness, contrast or
    /// gamma filter of the GPU driver.
    ///
    /// The minimum `mn` of [`Class::White`], [`Class::PureWhite`] and
    /// [`Class::Text`] moves down by `REFERENCE_WHITE − white` (at most
    /// 60), which follows a brightness offset exactly and the top of a
    /// contrast or gamma curve closely; the minimum top-hat response scales
    /// by the same ratio as the white level, which follows a contrast
    /// change. At or above [`ClassParams::REFERENCE_WHITE`] nothing
    /// changes, so unfiltered and brightened frames keep the tuned
    /// thresholds.
    #[must_use]
    pub fn for_white_level(&self, white: u8) -> Self {
        let shift = Self::REFERENCE_WHITE
            .saturating_sub(white)
            .min(Self::MAX_WHITE_SHIFT);
        if shift == 0 {
            return *self;
        }
        let level = u16::from(Self::REFERENCE_WHITE - shift);
        let tophat = u16::from(self.text_min_tophat) * level / u16::from(Self::REFERENCE_WHITE);
        Self {
            white_min: self.white_min.saturating_sub(shift),
            pure_white_min: self.pure_white_min.saturating_sub(shift),
            text_min_mn: self.text_min_mn.saturating_sub(shift),
            // `level < REFERENCE_WHITE`, so the result is below the old
            // minimum and fits.
            text_min_tophat: u8::try_from(tophat).unwrap_or(self.text_min_tophat),
            ..*self
        }
    }

    /// The chroma of white HUD text the chroma windows were tuned for: the
    /// median chroma of the brightest kill-feed name strokes is 6 to 13
    /// in unfiltered, greyscale and gamma-filtered recordings (measured).
    pub const REFERENCE_CHROMA: u8 = 16;

    /// The chroma windows widen by at most this much.
    const MAX_CHROMA_SHIFT: u8 = 40;

    /// The thresholds for a frame whose white HUD text reaches `white`
    /// with the median chroma `chroma` ([`ClassParams::for_white_level`]
    /// for the brightness): a colour tint of the GPU driver (a Freestyle
    /// or colour balance preset) gives every white a colour cast, which
    /// the fixed chroma windows of [`Class::White`], [`Class::PureWhite`]
    /// and [`Class::Text`] reject (measured on a real kill feed under a
    /// colour balance tint: chroma 49 instead of 13, the weapon icon and
    /// killer names lost). The windows widen by `chroma −
    /// REFERENCE_CHROMA`, at most 40; the team colours' chroma stays far
    /// above them.
    #[must_use]
    pub fn for_white_point(&self, white: u8, chroma: u8) -> Self {
        let level = self.for_white_level(white);
        let extra = chroma
            .saturating_sub(Self::REFERENCE_CHROMA)
            .min(Self::MAX_CHROMA_SHIFT);
        Self {
            white_max_chroma: level.white_max_chroma.saturating_add(extra),
            pure_white_max_chroma: level.pure_white_max_chroma.saturating_add(extra),
            text_max_chroma: level.text_max_chroma.saturating_add(extra),
            ..level
        }
    }

    /// The colour flags of one pixel (everything except [`Class::Text`],
    /// which needs the neighbourhood).
    pub fn colour_flags(&self, rgb: [u8; 3]) -> u8 {
        let v = rgb[0].max(rgb[1]).max(rgb[2]);
        let mn = rgb[0].min(rgb[1]).min(rgb[2]);
        let diff = v - mn;
        self.classify([hue(rgb, v, diff), saturation(v, diff), v], mn, diff)
    }

    /// The colour flags from HSV, `min(R, G, B)` and the chroma.
    ///
    /// Branch-free (`&` and `|` on booleans): the classes of textured
    /// gameplay are unpredictable, and the comparisons vectorise.
    #[inline]
    #[allow(
        clippy::needless_bitwise_bool,
        reason = "non-short-circuit operators keep the classification branch-free"
    )]
    fn classify(&self, [h, s, v]: [u8; 3], mn: u8, diff: u8) -> u8 {
        let white = (mn >= self.white_min) & (diff <= self.white_max_chroma);
        let pure = (mn >= self.pure_white_min) & (diff <= self.pure_white_max_chroma);
        let teal = in_hue_range(h, self.teal_hue) & (s >= self.teal_min_s) & (v >= self.teal_min_v);
        let red = in_hue_range(h, (self.red_hue_min, self.red_hue_max))
            & (s >= self.red_min_s)
            & (v >= self.red_min_v);
        let bar = (teal & (v >= self.bar_teal_min_v) & (s >= self.bar_teal_min_s))
            | (red & (v >= self.bar_red_min_v));
        Class::Teal.bit_if(teal)
            | Class::Red.bit_if(red)
            | Class::White.bit_if(white)
            | Class::PureWhite.bit_if(pure)
            | Class::Bar.bit_if(bar)
    }
}

/// Whether hue `h` lies in the inclusive circular range `start ..= end`
/// (OpenCV hues, `0..180`): `start ≤ h ≤ end`, or, for a range with
/// `start > end` that wraps around 0, `h ≥ start` or `h ≤ end`.
///
/// Branch-free like [`ClassParams::colour_flags`]: for a range that does
/// not wrap the second term is always false, for one that wraps the first.
#[inline]
#[allow(
    clippy::needless_bitwise_bool,
    reason = "non-short-circuit operators keep the classification branch-free"
)]
pub const fn in_hue_range(h: u8, (start, end): (u8, u8)) -> bool {
    let wraps = start > end;
    ((h >= start) & (h <= end)) | (wraps & ((h >= start) | (h <= end)))
}

/// Per-row buffers of [`PixelClasses::compute`].
///
/// The row is processed in passes over plain arrays, so that the compiler
/// vectorises everything except the two divisor-table lookups: about three
/// times faster than classifying pixel by pixel.
#[derive(Debug, Clone, Default)]
struct RowScratch {
    /// `max(R, G, B)`.
    v: Vec<u8>,
    /// Numerator of OpenCV's hue formula.
    num: Vec<i16>,
    /// `SDIV[v]`.
    sdiv: Vec<i32>,
    /// `HDIV[chroma]`.
    hdiv: Vec<i32>,
}

/// The output rows of [`RowScratch::classify_row`], all of the input's
/// length.
struct RowOut<'a> {
    mn: &'a mut [u8],
    chroma: &'a mut [u8],
    hue: &'a mut [u8],
    sat: &'a mut [u8],
    flags: &'a mut [u8],
}

impl RowScratch {
    /// Classifies one row: fills `mn`, `chroma`, hue, saturation and the
    /// colour flags.
    #[allow(
        clippy::needless_range_loop,
        reason = "indexing parallel arrays keeps each pass vectorisable"
    )]
    fn classify_row(
        &mut self,
        pixels: &[[u8; 3]],
        params: &ClassParams,
        RowOut {
            mn,
            chroma,
            hue,
            sat,
            flags,
        }: RowOut<'_>,
    ) {
        let n = pixels.len();
        self.v.resize(n, 0);
        self.num.resize(n, 0);
        self.sdiv.resize(n, 0);
        self.hdiv.resize(n, 0);
        let (mn, chroma, flags) = (&mut mn[..n], &mut chroma[..n], &mut flags[..n]);
        let (v, num) = (&mut self.v[..n], &mut self.num[..n]);
        for i in 0..n {
            let [r, g, b] = pixels[i];
            let hi = r.max(g).max(b);
            let lo = r.min(g).min(b);
            v[i] = hi;
            mn[i] = lo;
            chroma[i] = hi - lo;
            let vr = -i16::from(hi == r);
            let vg = -i16::from(hi == g);
            let [r, g, b, d] = [r, g, b, hi - lo].map(i16::from);
            num[i] = (vr & (g - b)) + (!vr & ((vg & (b - r + 2 * d)) + (!vg & (r - g + 4 * d))));
        }
        for (dst, &hi) in self.sdiv.iter_mut().zip(v.iter()) {
            *dst = SDIV[usize::from(hi)];
        }
        for (dst, &d) in self.hdiv.iter_mut().zip(chroma.iter()) {
            *dst = HDIV[usize::from(d)];
        }
        let (sdiv, hdiv) = (&self.sdiv[..n], &self.hdiv[..n]);
        let (h, s) = (&mut hue[..n], &mut sat[..n]);

        let round = 1 << (HSV_SHIFT - 1);
        for i in 0..n {
            s[i] = ((i32::from(chroma[i]) * sdiv[i] + round) >> HSV_SHIFT) as u8;
            let raw = (i32::from(num[i]) * hdiv[i] + round) >> HSV_SHIFT;
            h[i] = (raw + 180 * i32::from(raw < 0)) as u8;
        }
        for i in 0..n {
            flags[i] = params.classify([h[i], s[i], v[i]], mn[i], chroma[i]);
        }
    }
}

/// The pixel classes of one crop, with the channels they are built from.
///
/// Reuse one value for every frame: [`PixelClasses::compute`] keeps its
/// buffers.
#[derive(Debug, Clone, Default)]
pub struct PixelClasses {
    width: u32,
    height: u32,
    mn: GrayImage,
    chroma: GrayImage,
    hue: GrayImage,
    sat: GrayImage,
    tophat: GrayImage,
    flags: Vec<u8>,
    rows: RowScratch,
    scratch: MorphScratch,
    /// `mn` of the area [`PixelClasses::compute_rect`] runs the top-hat on.
    part_mn: GrayImage,
    /// The top-hat of `part_mn`.
    part_tophat: GrayImage,
}

impl PixelClasses {
    /// Empty classes; call [`PixelClasses::compute`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Classifies every pixel of `view`.
    pub fn compute(&mut self, view: &RgbView<'_>, params: &ClassParams) {
        let (w, h) = (view.width(), view.height());
        self.width = w;
        self.height = h;
        for plane in [&mut self.mn, &mut self.chroma, &mut self.hue, &mut self.sat] {
            plane.resize(w, h);
        }
        self.flags.clear();
        self.flags.resize(w as usize * h as usize, 0);
        for y in 0..h {
            let row = view.row(y);
            let range = y as usize * w as usize..(y as usize + 1) * w as usize;
            let out = RowOut {
                mn: &mut self.mn.data_mut()[range.clone()],
                chroma: &mut self.chroma.data_mut()[range.clone()],
                hue: &mut self.hue.data_mut()[range.clone()],
                sat: &mut self.sat.data_mut()[range.clone()],
                flags: &mut self.flags[range],
            };
            self.rows.classify_row(row.as_chunks::<3>().0, params, out);
        }
        let k = params.tophat_size;
        white_tophat(&self.mn, k, k, &mut self.tophat, &mut self.scratch);
        let text = self
            .tophat
            .data()
            .iter()
            .zip(self.mn.data())
            .zip(self.chroma.data());
        for (flag, ((&th, &mn), &chroma)) in self.flags.iter_mut().zip(text) {
            if th >= params.text_min_tophat
                && mn >= params.text_min_mn
                && chroma <= params.text_max_chroma
            {
                *flag |= Class::Text.bit();
            }
        }
    }

    /// Classifies only the pixels of `rect` (clipped to the view) and
    /// leaves the rest of the planes as they are.
    ///
    /// Detectors that need a few bands of a large crop use this instead of
    /// [`PixelClasses::compute`]: the planes keep the size of `view`, and
    /// inside `rect` every channel and flag is exactly what `compute` gives
    /// for the whole view (the top-hat sees the same neighbourhood, and the
    /// borders of the view stay borders). In a margin of
    /// `2 × (tophat_size / 2)` pixels around `rect` the colour flags, `mn` and
    /// the chroma (and hue and saturation) are set as well, but the
    /// [`Class::Text`] flag is cleared
    /// and the top-hat is unspecified. Pixels further away keep the values
    /// of earlier calls (0 after the view size changed), so a caller that
    /// classifies several areas of one frame must not let the margin of
    /// one area overlap another area it still reads.
    pub fn compute_rect(&mut self, view: &RgbView<'_>, rect: RectI, params: &ClassParams) {
        let (w, h) = (view.width(), view.height());
        if (w, h) != (self.width, self.height) {
            self.width = w;
            self.height = h;
            for plane in [
                &mut self.mn,
                &mut self.chroma,
                &mut self.hue,
                &mut self.sat,
                &mut self.tophat,
            ] {
                plane.resize(w, h);
                plane.data_mut().fill(0);
            }
            self.flags.clear();
            self.flags.resize(w as usize * h as usize, 0);
        }
        let Some(inner) = RectI::new(0, 0, w as i32, h as i32)
            .ok()
            .and_then(|b| b.intersection(&rect))
        else {
            return;
        };
        let k = params.tophat_size;
        // The opening at a pixel reads the source up to `2 × (k / 2)`
        // pixels away (erosion, then dilation, both anchored at `k / 2`).
        let margin = 2 * (k / 2) as i32;
        let outer = RectI {
            x0: (inner.x0 - margin).max(0),
            y0: (inner.y0 - margin).max(0),
            x1: (inner.x1 + margin).min(w as i32),
            y1: (inner.y1 + margin).min(h as i32),
        };
        let (ox0, ox1) = (outer.x0 as usize, outer.x1 as usize);
        let width = w as usize;
        self.part_mn.resize(outer.width(), outer.height());
        for y in outer.y0..outer.y1 {
            let start = y as usize * width;
            let range = start + ox0..start + ox1;
            let pixels = &view.row(y as u32).as_chunks::<3>().0[ox0..ox1];
            let out = RowOut {
                mn: &mut self.mn.data_mut()[range.clone()],
                chroma: &mut self.chroma.data_mut()[range.clone()],
                hue: &mut self.hue.data_mut()[range.clone()],
                sat: &mut self.sat.data_mut()[range.clone()],
                flags: &mut self.flags[range.clone()],
            };
            self.rows.classify_row(pixels, params, out);
            let part_row = (y - outer.y0) as usize * (ox1 - ox0);
            self.part_mn.data_mut()[part_row..part_row + ox1 - ox0]
                .copy_from_slice(&self.mn.data()[range]);
        }
        white_tophat(
            &self.part_mn,
            k,
            k,
            &mut self.part_tophat,
            &mut self.scratch,
        );
        let (ix0, ix1) = (inner.x0 as usize, inner.x1 as usize);
        for y in inner.y0..inner.y1 {
            let start = y as usize * width;
            let part = self.part_tophat.row((y - outer.y0) as u32);
            let tophat = &part[ix0 - ox0..ix1 - ox0];
            self.tophat.data_mut()[start + ix0..start + ix1].copy_from_slice(tophat);
            let text = tophat
                .iter()
                .zip(&self.mn.data()[start + ix0..start + ix1])
                .zip(&self.chroma.data()[start + ix0..start + ix1]);
            for (flag, ((&th, &mn), &chroma)) in
                self.flags[start + ix0..start + ix1].iter_mut().zip(text)
            {
                if th >= params.text_min_tophat
                    && mn >= params.text_min_mn
                    && chroma <= params.text_max_chroma
                {
                    *flag |= Class::Text.bit();
                }
            }
        }
    }

    /// Width of the classified crop.
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Height of the classified crop.
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// `min(R, G, B)` per pixel (the channel the kill-feed tracker
    /// correlates on).
    pub const fn mn(&self) -> &GrayImage {
        &self.mn
    }

    /// `max(R, G, B) − min(R, G, B)` per pixel.
    pub const fn chroma(&self) -> &GrayImage {
        &self.chroma
    }

    /// OpenCV's 8-bit hue (`0..180`) per pixel.
    pub const fn hue(&self) -> &GrayImage {
        &self.hue
    }

    /// OpenCV's 8-bit saturation per pixel. The value (`max(R, G, B)`) is
    /// [`PixelClasses::mn`] plus [`PixelClasses::chroma`].
    pub const fn saturation(&self) -> &GrayImage {
        &self.sat
    }

    /// The white top-hat of [`PixelClasses::mn`] (spec `TH7`).
    pub const fn tophat(&self) -> &GrayImage {
        &self.tophat
    }

    /// The class flags per pixel, row after row.
    pub fn flags(&self) -> &[u8] {
        &self.flags
    }

    /// Whether pixel `(x, y)` belongs to `class`.
    ///
    /// # Panics
    ///
    /// Panics if the pixel is outside the crop.
    pub fn has(&self, x: u32, y: u32, class: Class) -> bool {
        self.flags[y as usize * self.width as usize + x as usize] & class.bit() != 0
    }

    /// The mask of one class.
    pub fn mask(&self, class: Class) -> BitMask<'_> {
        // `flags` holds width × height entries after `compute`; before it
        // the size is 0 × 0.
        BitMask::from_plane(&self.flags, class.bit(), self.width, self.height)
    }
}

#[cfg(test)]
mod tests {
    use blurrd_core::RectI;

    use super::{Class, ClassParams, PixelClasses, hsv, in_hue_range};
    use crate::image::RgbView;

    #[test]
    fn hsv_matches_hand_values() {
        assert_eq!(hsv([0, 0, 0]), [0, 0, 0]);
        assert_eq!(hsv([255, 255, 255]), [0, 0, 255]);
        assert_eq!(hsv([255, 0, 0]), [0, 255, 255]);
        assert_eq!(hsv([0, 255, 0]), [60, 255, 255]);
        assert_eq!(hsv([0, 0, 255]), [120, 255, 255]);
        // Measured bar colours (port spec section 0).
        assert_eq!(hsv([100, 186, 162]), [82, 118, 186]);
        assert_eq!(hsv([230, 98, 93]), [1, 152, 230]);
    }

    #[test]
    fn classifies_bars_and_whites() {
        let p = ClassParams::default();
        let teal = p.colour_flags([100, 186, 162]);
        assert_eq!(teal, Class::Teal.bit() | Class::Bar.bit());
        let red = p.colour_flags([230, 98, 93]);
        assert_eq!(red, Class::Red.bit() | Class::Bar.bit());
        let dim_red = p.colour_flags([140, 60, 55]);
        assert_eq!(dim_red, Class::Red.bit(), "red but too dark for a bar");
        let white = p.colour_flags([250, 250, 250]);
        assert_eq!(white, Class::White.bit() | Class::PureWhite.bit());
        assert_eq!(p.colour_flags([180, 180, 200]), Class::White.bit());
        assert_eq!(p.colour_flags([90, 90, 90]), 0);
    }

    #[test]
    fn a_tinted_white_point_widens_the_chroma_windows() {
        let p = ClassParams::default();
        // Unfiltered and greyscale: nothing changes.
        assert_eq!(p.for_white_point(240, 13), p);
        assert_eq!(p.for_white_point(255, 0), p);
        // A colour balance tint (measured: white level 202, chroma 49):
        // the whitened weapon icon and the tinted names are white again.
        let tint = p.for_white_point(202, 49);
        assert_eq!(tint.white_max_chroma, p.white_max_chroma + 33);
        assert_eq!(tint.pure_white_max_chroma, p.pure_white_max_chroma + 33);
        assert_eq!(tint.text_max_chroma, p.text_max_chroma + 33);
        assert_eq!(tint.white_min, p.for_white_level(202).white_min);
        assert_ne!(
            tint.colour_flags([222, 240, 196]) & Class::PureWhite.bit(),
            0
        );
        assert_eq!(p.colour_flags([222, 240, 196]) & Class::PureWhite.bit(), 0);
        // The team colours stay out of the white classes.
        let white = Class::White.bit() | Class::PureWhite.bit();
        assert_eq!(tint.colour_flags([96, 220, 190]) & white, 0);
        assert_eq!(tint.colour_flags([235, 85, 90]) & white, 0);
        // The widening is bounded.
        let far = p.for_white_point(200, 200);
        assert_eq!(far.white_max_chroma, p.white_max_chroma + 40);
    }

    #[test]
    fn a_lower_white_level_lowers_the_brightness_thresholds() {
        let p = ClassParams::default();
        assert_eq!(p.for_white_level(ClassParams::REFERENCE_WHITE), p);
        assert_eq!(p.for_white_level(250), p);
        // A brightness offset of about −25.
        let dark = p.for_white_level(200);
        assert_eq!(
            (dark.white_min, dark.pure_white_min, dark.text_min_mn),
            (145, 190, 140)
        );
        assert_eq!(dark.text_min_tophat, 31);
        assert_eq!((dark.teal_hue, dark.red_min_v), (p.teal_hue, p.red_min_v));
        // The shift is capped.
        let black = p.for_white_level(0);
        assert_eq!(black.text_min_mn, 105);
        assert_eq!(black.text_min_tophat, 25);
    }

    #[test]
    fn finds_thin_text_strokes() {
        // A 2 px bright vertical stroke on a dark 16 × 12 crop.
        let (w, h) = (16u32, 12u32);
        let mut data = vec![40u8; (3 * w * h) as usize];
        for y in 2..10 {
            for x in 7..9 {
                let i = (3 * (y * w + x)) as usize;
                data[i..i + 3].copy_from_slice(&[240, 240, 240]);
            }
        }
        let view = RgbView::new(&data, w, h, 3 * w as usize).unwrap();
        let mut classes = PixelClasses::new();
        classes.compute(&view, &ClassParams::default());
        assert!(classes.has(7, 5, Class::Text) && classes.has(8, 9, Class::Text));
        assert!(!classes.has(6, 5, Class::Text) && !classes.has(7, 1, Class::Text));
        assert_eq!(classes.tophat().get(7, 5), 200);
        let mask = classes.mask(Class::Text);
        assert!(mask.get(8, 2) && !mask.get(0, 0));
    }

    /// Classifies every colour of `colours` with the row path and checks
    /// each pixel against [`ClassParams::colour_flags`].
    fn check_rows(colours: &[[u8; 3]], params: &ClassParams) {
        let width = 1024u32;
        let height = colours.len().div_ceil(width as usize) as u32;
        let mut data: Vec<u8> = colours.iter().flatten().copied().collect();
        data.resize((3 * width * height) as usize, 0);
        let view = RgbView::new(&data, width, height, 3 * width as usize).unwrap();
        let mut classes = PixelClasses::new();
        classes.compute(&view, params);
        for (i, &rgb) in colours.iter().enumerate() {
            let text = Class::Text.bit();
            assert_eq!(
                classes.flags()[i] & !text,
                params.colour_flags(rgb),
                "{rgb:?}"
            );
            assert_eq!(classes.mn().data()[i], rgb.iter().copied().min().unwrap());
        }
    }

    #[test]
    fn row_classification_matches_single_pixels() {
        // Every combination of 32 levels plus a pseudo-random sample, with
        // the default and with shifted hue ranges.
        let levels: Vec<u8> = (0..32).map(|i| (i * 255 / 31) as u8).collect();
        let mut colours: Vec<[u8; 3]> = Vec::new();
        for &r in &levels {
            for &g in &levels {
                for &b in &levels {
                    colours.push([r, g, b]);
                }
            }
        }
        let mut x = 0x2545_F491_4F6C_DD1Du64;
        for _ in 0..65_536 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            colours.push([x as u8, (x >> 8) as u8, (x >> 16) as u8]);
        }
        check_rows(&colours, &ClassParams::default());
        let shifted = ClassParams {
            teal_hue: (100, 130),
            red_hue_max: 25,
            red_hue_min: 150,
            ..ClassParams::default()
        };
        check_rows(&colours, &shifted);
        // Learned ranges: a red that does not wrap, a teal that does.
        let learned = ClassParams {
            teal_hue: (172, 8),
            red_hue_min: 20,
            red_hue_max: 45,
            ..ClassParams::default()
        };
        check_rows(&colours, &learned);
    }

    #[test]
    fn hue_ranges_may_wrap_around_red() {
        for h in 0..180u8 {
            assert_eq!(in_hue_range(h, (66, 96)), (66..=96).contains(&h), "{h}");
            assert_eq!(in_hue_range(h, (170, 10)), h >= 170 || h <= 10, "{h}");
        }
        assert!(in_hue_range(40, (40, 40)) && !in_hue_range(41, (40, 40)));
        // The default red range is the port spec's `h ≤ 10 or h ≥ 170`.
        let p = ClassParams::default();
        let red = |rgb| p.colour_flags(rgb) & Class::Red.bit() != 0;
        assert!(red([230, 40, 40]) && red([230, 40, 70]) && !red([230, 140, 40]));
    }

    #[test]
    #[ignore = "classifies all 16.7 million colours; run in release builds"]
    fn exhaustive_row_classification_matches_single_pixels() {
        let colours: Vec<[u8; 3]> = (0..1u32 << 24)
            .map(|i| [(i >> 16) as u8, (i >> 8) as u8, i as u8])
            .collect();
        check_rows(&colours, &ClassParams::default());
    }

    #[test]
    fn classifying_a_rect_matches_the_whole_crop_inside_it() {
        let (w, h) = (61u32, 47u32);
        let mut x = 0x9E37_79B9_7F4A_7C15u64;
        let data: Vec<u8> = (0..3 * w * h)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                // Mostly bright pixels, so text strokes occur.
                if x.is_multiple_of(3) {
                    (x >> 8) as u8
                } else {
                    160 + (x >> 16) as u8 % 96
                }
            })
            .collect();
        let view = RgbView::new(&data, w, h, 3 * w as usize).unwrap();
        for k in [5, 7, 8] {
            let params = ClassParams {
                tophat_size: k,
                text_min_tophat: 10,
                ..ClassParams::default()
            };
            let mut whole = PixelClasses::new();
            whole.compute(&view, &params);
            let mut part = PixelClasses::new();
            let rects = [
                RectI::new(20, 15, 40, 30).unwrap(),
                RectI::new(0, 0, 9, 47).unwrap(),
                RectI::new(50, 40, 70, 60).unwrap(),
                RectI::new(-5, 10, 3, 12).unwrap(),
            ];
            for rect in rects {
                part.compute_rect(&view, rect, &params);
                let inner = rect
                    .intersection(&RectI::new(0, 0, 61, 47).unwrap())
                    .unwrap();
                let mut text = 0;
                for y in inner.y0 as u32..inner.y1 as u32 {
                    for x in inner.x0 as u32..inner.x1 as u32 {
                        let i = (y * w + x) as usize;
                        assert_eq!(part.flags()[i], whole.flags()[i], "k {k} at ({x}, {y})");
                        assert_eq!(part.tophat().get(x, y), whole.tophat().get(x, y));
                        assert_eq!(part.mn().get(x, y), whole.mn().get(x, y));
                        assert_eq!(part.hue().get(x, y), whole.hue().get(x, y));
                        assert_eq!(part.saturation().get(x, y), whole.saturation().get(x, y));
                        text += u32::from(whole.has(x, y, Class::Text));
                    }
                }
                assert!(
                    text > 0 || inner.area() < 100,
                    "k {k}: the test needs text strokes"
                );
            }
            assert_eq!((part.width(), part.height()), (w, h));
        }
    }

    #[test]
    fn hue_and_saturation_planes_hold_opencv_hsv() {
        let colours: [[u8; 3]; 6] = [
            [100, 186, 162],
            [230, 98, 93],
            [245, 245, 245],
            [0, 0, 0],
            [70, 130, 230],
            [222, 80, 170],
        ];
        let data: Vec<u8> = colours.iter().flatten().copied().collect();
        let view = RgbView::new(&data, 6, 1, 18).unwrap();
        let mut classes = PixelClasses::new();
        classes.compute(&view, &ClassParams::default());
        for (x, &rgb) in (0..).zip(&colours) {
            let [h, s, v] = hsv(rgb);
            assert_eq!(classes.hue().get(x, 0), h, "{rgb:?}");
            assert_eq!(classes.saturation().get(x, 0), s, "{rgb:?}");
            assert_eq!(classes.mn().get(x, 0) + classes.chroma().get(x, 0), v);
        }
    }

    #[test]
    fn empty_classes_have_an_empty_mask() {
        let classes = PixelClasses::new();
        assert_eq!(classes.mask(Class::Bar).width(), 0);
    }
}
