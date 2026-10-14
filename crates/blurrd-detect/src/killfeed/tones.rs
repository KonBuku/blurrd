//! Bar colours learned from the frame itself.
//!
//! The pixel classes know the default bar colours (teal and red, port spec
//! section 2), but colour-blind modes, custom team colours and GPU filters
//! (hue shifts, digital vibrance, saturation, gamma) move them out of those
//! ranges. A kill-feed row has structure that does not depend on colour:
//! a right-aligned white name on a uniform bar next to the victim portrait.
//! Where that structure is present but the default colours do not explain
//! the bar, the bar's colour is learned from the pixels around the name
//! ([`learn`]) and used like a class ([`Tone::matches`]).
//!
//! Thresholds are relative to the learned colour, so a desaturated or
//! darkened bar matches as well as the original.

use blurrd_core::RectI;

use super::FEED_ROI;
use crate::classes::{Class, PixelClasses};

/// Hues within this distance (OpenCV units, `0..180`, circular) belong to
/// one tone (port spec 10.7). Bar pixels spread about ±3 around their
/// mode; gradients, lighter shades of the same team colour, text
/// anti-aliasing and video noise add more.
const HUE_TOLERANCE: u8 = 12;
/// Pixels below this saturation carry no reliable hue.
const MIN_SAMPLE_S: u8 = 25;
/// Pixels below this value carry no reliable hue.
const MIN_SAMPLE_V: u8 = 50;
/// A pixel matches a tone if its saturation is at least this share of the
/// tone's median saturation (the edge of the bar and anti-aliased text
/// pixels are less saturated than its centre).
const S_SHARE: f32 = 0.4;
/// The same for the value.
const V_SHARE: f32 = 0.5;
/// Lower bound of a tone's minimum saturation.
const MIN_TONE_S: u8 = 20;
/// Lower bound of a tone's minimum value.
const MIN_TONE_V: u8 = 40;
/// The two team colours differ at least this much in hue (OpenCV units;
/// teal and red are 80 apart). A closer second colour is the same bar in
/// another shade: the victim side of a one-colour row is lighter than the
/// killer side, and bars have gradients.
pub(super) const MIN_TEAM_HUE_GAP: u8 = 25;

/// A learned bar colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct Tone {
    /// Mode of the hue.
    hue: u8,
    /// Minimum saturation of a matching pixel.
    min_s: u8,
    /// Minimum value of a matching pixel.
    min_v: u8,
}

impl Tone {
    /// Whether a pixel with hue `h`, saturation `s` and value `v` has this
    /// colour.
    pub(super) fn matches(self, h: u8, s: u8, v: u8) -> bool {
        hue_distance(h, self.hue) <= HUE_TOLERANCE && s >= self.min_s && v >= self.min_v
    }

    /// Whether two tones describe the same colour.
    pub(super) fn same_colour(self, other: Self) -> bool {
        hue_distance(self.hue, other.hue) <= HUE_TOLERANCE
    }

    /// The tone that matches the pixels of both tones of one colour: the
    /// hue of `self` with the lower saturation and value bounds.
    pub(super) fn widened(self, other: Self) -> Self {
        Self {
            hue: self.hue,
            min_s: self.min_s.min(other.min_s),
            min_v: self.min_v.min(other.min_v),
        }
    }

    /// The circular distance of the two tones' hues (`0..=90`).
    pub(super) fn hue_gap(self, other: Self) -> u8 {
        hue_distance(self.hue, other.hue)
    }

    /// A tone from its parts.
    #[cfg(test)]
    pub(super) const fn new(hue: u8, min_s: u8, min_v: u8) -> Self {
        Self { hue, min_s, min_v }
    }

    /// The tone's hue (OpenCV units, `0..180`).
    pub(super) const fn hue(self) -> u8 {
        self.hue
    }

    /// The minimum saturation of a matching pixel.
    pub(super) const fn min_s(self) -> u8 {
        self.min_s
    }

    /// The minimum value of a matching pixel.
    pub(super) const fn min_v(self) -> u8 {
        self.min_v
    }

    /// Adds the pixels of each column of `rect` that have this colour to
    /// `counts` (one entry per column of `rect`).
    pub(super) fn count_columns(self, classes: &PixelClasses, rect: RectI, counts: &mut [u32]) {
        for row in plane_rows(classes, rect) {
            for (n, i) in counts.iter_mut().zip(0..row.hue.len()) {
                *n += u32::from(self.matches(row.hue[i], row.sat[i], row.value(i)));
            }
        }
    }
}

/// The planes of one crop row of [`PixelClasses`], sliced to the columns
/// of a rectangle.
pub(super) struct PlaneRow<'a> {
    /// Hue.
    pub(super) hue: &'a [u8],
    /// Saturation.
    pub(super) sat: &'a [u8],
    /// `min(R, G, B)`.
    mn: &'a [u8],
    /// `max(R, G, B) − min(R, G, B)`.
    chroma: &'a [u8],
    /// Class flags.
    pub(super) flags: &'a [u8],
}

impl PlaneRow<'_> {
    /// The value (`max(R, G, B)`) of pixel `i`.
    pub(super) fn value(&self, i: usize) -> u8 {
        self.mn[i].saturating_add(self.chroma[i])
    }
}

/// The rows of reference rectangle `rect` (inside [`FEED_ROI`]).
pub(super) fn plane_rows(
    classes: &PixelClasses,
    rect: RectI,
) -> impl Iterator<Item = PlaneRow<'_>> {
    let width = classes.width() as usize;
    let (x0, x1) = (
        (rect.x0 - FEED_ROI.x0) as usize,
        (rect.x1 - FEED_ROI.x0) as usize,
    );
    (rect.y0 - FEED_ROI.y0..rect.y1 - FEED_ROI.y0).map(move |y| {
        let range = y as usize * width + x0..y as usize * width + x1;
        PlaneRow {
            hue: &classes.hue().data()[range.clone()],
            sat: &classes.saturation().data()[range.clone()],
            mn: &classes.mn().data()[range.clone()],
            chroma: &classes.chroma().data()[range.clone()],
            flags: &classes.flags()[range],
        }
    })
}

/// Circular distance of two OpenCV hues (`0..180`).
pub(super) fn hue_distance(a: u8, b: u8) -> u8 {
    let d = a.abs_diff(b);
    d.min(180u8.saturating_sub(d))
}

/// What [`learn`] samples and how uniform the colour must be.
#[derive(Debug, Clone, Copy)]
pub(super) struct Sample {
    /// Reference rectangle to sample.
    pub(super) rect: RectI,
    /// Pixels of this colour are skipped (the other side of a row).
    pub(super) exclude: Option<Tone>,
    /// The tone needs at least this share of the sampled pixels.
    pub(super) min_share: f32,
    /// ... and at least this many pixels.
    pub(super) min_pixels: u32,
    /// Whether pixels without a reliable hue count in the share: `true`
    /// asks for a uniform coloured area, `false` for the main colour among
    /// the coloured pixels.
    pub(super) count_grey: bool,
}

/// Learns the dominant bar colour of the reference rectangle in `sample`,
/// skipping white and text pixels.
///
/// Returns `None` if no hue reaches the required share. `classes` must
/// hold the colour planes of the rectangle.
pub(super) fn learn(classes: &PixelClasses, sample: &Sample) -> Option<Tone> {
    let skip = Class::White.bit() | Class::Text.bit();
    // Which pixels are sampled: not white or text, not excluded, and with
    // a reliable hue (`coloured`).
    let sampled = |row: &PlaneRow<'_>, i: usize| {
        let (h, s, v) = (row.hue[i], row.sat[i], row.value(i));
        let usable = row.flags[i] & skip == 0 && sample.exclude.is_none_or(|t| !t.matches(h, s, v));
        (usable, s >= MIN_SAMPLE_S && v >= MIN_SAMPLE_V)
    };
    let mut hues = [0u32; 180];
    let mut considered = 0u32;
    for row in plane_rows(classes, sample.rect) {
        for i in 0..row.hue.len() {
            let (usable, coloured) = sampled(&row, i);
            considered += u32::from(usable && (coloured || sample.count_grey));
            if usable && coloured {
                hues[usize::from(row.hue[i])] += 1;
            }
        }
    }
    // Pixels within the tolerance of each hue, as a sliding circular sum.
    let tol = usize::from(HUE_TOLERANCE);
    let mut window: u32 = (0..=2 * tol).map(|k| hues[(k + 180 - tol) % 180]).sum();
    let (mut mode, mut count) = (0u8, window);
    for h in 1..180usize {
        window += hues[(h + tol) % 180];
        window -= hues[(h + 179 - tol) % 180];
        if window > count {
            (mode, count) = (h as u8, window);
        }
    }
    // The first hue whose window holds the most pixels can sit at the
    // window's edge of a narrow peak: centre the tone on the mean hue of
    // its window, and count again around that centre.
    let mode = usize::from(mode);
    let (weighted, total) = (0..=2 * tol).fold((0i64, 0i64), |(w, t), k| {
        let n = i64::from(hues[(mode + 180 + k - tol) % 180]);
        (w + n * (k as i64 - tol as i64), t + n)
    });
    let offset = if total > 0 { weighted / total } else { 0 };
    let mode = (mode as i64 + offset).rem_euclid(180) as usize;
    let count: u32 = (0..=2 * tol)
        .map(|k| hues[(mode + 180 + k - tol) % 180])
        .sum();
    let mode = mode as u8;
    if count < sample.min_pixels || (count as f32) < sample.min_share * considered as f32 {
        return None;
    }
    // Median saturation and value of the pixels of the mode.
    let (mut s_hist, mut v_hist) = ([0u32; 256], [0u32; 256]);
    for row in plane_rows(classes, sample.rect) {
        for i in 0..row.hue.len() {
            let (usable, coloured) = sampled(&row, i);
            if usable && coloured && hue_distance(row.hue[i], mode) <= HUE_TOLERANCE {
                s_hist[usize::from(row.sat[i])] += 1;
                v_hist[usize::from(row.value(i))] += 1;
            }
        }
    }
    let scaled = |hist: &[u32; 256], share: f32, floor: u8| {
        let median = median(hist, count);
        floor.max((f32::from(median) * share) as u8)
    };
    Some(Tone {
        hue: mode,
        min_s: scaled(&s_hist, S_SHARE, MIN_TONE_S),
        min_v: scaled(&v_hist, V_SHARE, MIN_TONE_V),
    })
}

/// The median of `count` values given as a histogram.
fn median(hist: &[u32; 256], count: u32) -> u8 {
    let half = count.div_ceil(2);
    let mut seen = 0;
    for (value, &n) in (0..=255u8).zip(hist) {
        seen += n;
        if seen >= half {
            return value;
        }
    }
    u8::MAX
}

#[cfg(test)]
mod tests {
    use blurrd_core::RectI;

    use super::{Sample, hue_distance, learn};
    use crate::classes::{ClassParams, PixelClasses};
    use crate::image::RgbView;
    use crate::killfeed::FEED_ROI;

    /// Classes of a feed crop: mid-grey with a bar of `colour` (with a
    /// lighter left half if `shade`) over reference columns 1700..1838
    /// and rows 100..133.
    fn crop(colour: [u8; 3], shade: bool) -> PixelClasses {
        let (w, h) = (FEED_ROI.width() as usize, FEED_ROI.height() as usize);
        let mut data = vec![128u8; 3 * w * h];
        for y in 100 - 80..133 - 80 {
            for x in 1700 - 660..1838 - 660 {
                let c = if shade && x < 1769 - 660 {
                    colour.map(|v| v.saturating_add((255 - v) / 3))
                } else {
                    colour
                };
                data[3 * (y * w + x)..3 * (y * w + x) + 3].copy_from_slice(&c);
            }
        }
        let view = RgbView::new(&data, w as u32, h as u32, 3 * w).unwrap();
        let mut classes = PixelClasses::new();
        classes.compute(&view, &ClassParams::default());
        classes
    }

    fn sample(x0: i32, x1: i32) -> Sample {
        Sample {
            rect: RectI::new(x0, 104, x1, 129).unwrap(),
            exclude: None,
            min_share: 0.6,
            min_pixels: 300,
            count_grey: true,
        }
    }

    #[test]
    fn hue_distance_wraps_around_red() {
        assert_eq!(hue_distance(2, 178), 4);
        assert_eq!(hue_distance(178, 2), 4);
        assert_eq!(hue_distance(80, 90), 10);
        assert_eq!(hue_distance(0, 90), 90);
        assert_eq!(hue_distance(45, 45), 0);
    }

    #[test]
    fn learns_a_uniform_bar_and_matches_darker_and_paler_pixels() {
        // A colour-blind stand-in blue (OpenCV hue 109).
        let classes = crop([70, 130, 230], false);
        let tone = learn(&classes, &sample(1790, 1838)).unwrap();
        assert!(tone.matches(109, 178, 230));
        // Desaturated and darkened by a filter, still the same bar.
        assert!(tone.matches(112, 100, 150));
        // Another hue or a grey pixel is not.
        assert!(!tone.matches(80, 178, 230));
        assert!(!tone.matches(109, 10, 230));
    }

    #[test]
    fn needs_a_uniform_colour() {
        // Grey background only: no reliable hue.
        let classes = crop([128, 128, 128], false);
        assert_eq!(learn(&classes, &sample(1790, 1838)), None);
        // Half bar, half grey: below the share.
        let classes = crop([70, 130, 230], false);
        assert_eq!(learn(&classes, &sample(1640, 1760)), None);
    }

    #[test]
    fn excludes_a_known_colour_and_widens_shades() {
        let classes = crop([222, 80, 170], true);
        let dark = learn(&classes, &sample(1790, 1838)).unwrap();
        let pale = learn(&classes, &sample(1700, 1760)).unwrap();
        assert!(dark.same_colour(pale) && dark.hue_gap(pale) <= 2);
        let wide = dark.widened(pale);
        assert!(wide.matches(dark.hue, pale.min_s, pale.min_v));
        // Everything in the bar has the excluded colour: nothing left.
        let rest = Sample {
            exclude: Some(wide),
            ..sample(1700, 1838)
        };
        assert_eq!(learn(&classes, &rest), None);
    }
}
