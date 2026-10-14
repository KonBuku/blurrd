//! Kill-feed bar hues learned per video (plan 7.3).
//!
//! Every frame can learn the bar colours of its rows ([`tones`]), but that
//! is the slow path: the rows are first scored with the default teal and
//! red, and only where those do not explain a bar is its colour learned.
//! A video keeps its two team colours (a colour-blind mode, custom team
//! colours or a GPU colour filter changes them for the whole recording),
//! so [`BarHueLearner`] collects the colour next to the victim name of the
//! rows of sampled frames. Over a video both teams die, so the victim
//! colours gather in two clusters at least
//! [`MIN_TEAM_HUE_GAP`](super::tones::MIN_TEAM_HUE_GAP) apart: the
//! learned [`BarHues`]. Their [`ClassParams`] replace the default hue
//! ranges, which turns the learned colours into the fast default path of
//! [`KillFeedDetector`](super::KillFeedDetector); per-frame learning stays
//! as the fallback for rows they do not explain.
//!
//! Videos whose team colours lie in the default ranges learn nothing
//! ([`BarHueLearner::hues`] is `None`), so they are read exactly as
//! before.

use blurrd_core::RectI;

use super::analysis::{VICTIM_TONE_PIXELS, VICTIM_TONE_SHARE};
use super::tones::{self, MIN_TEAM_HUE_GAP, Sample, Tone, hue_distance};
use super::{FeedRow, INNER_MARGIN, KillFeedDetector, ROW_H, VICTIM_COLOUR_X0, VICTIM_X1};
use crate::classes::{ClassParams, in_hue_range};

/// Victim colours within this hue distance (OpenCV units) belong to one
/// team colour: the tolerance of a learned tone.
const CLUSTER_TOLERANCE: u8 = 12;
/// A learned hue range reaches this far on both sides of its centre: the
/// half width of the default teal range (66 to 96).
const HALF_WIDTH: u8 = 15;
/// Each team colour needs at least this many rows...
const MIN_ROWS: usize = 6;
/// ... and at least this share of the rows sampled (the rest are rows
/// over a mixed background, or fading).
const MIN_SHARE: f32 = 0.15;
/// Rows kept for learning; later rows are not needed.
const MAX_SAMPLES: usize = 4096;
/// The centre of the default teal range: of the two learned colours, the
/// one closer to it replaces teal, the other red.
const TEAL_CENTRE: u8 = 81;

/// One team colour of a video.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TeamHue {
    /// Centre of the hue range (OpenCV units, `0..180`).
    pub hue: u8,
    /// Minimum saturation of a bar pixel of this colour.
    pub min_s: u8,
    /// Minimum value of a bar pixel of this colour.
    pub min_v: u8,
}

impl TeamHue {
    /// The inclusive circular hue range of the colour.
    pub const fn range(self) -> (u8, u8) {
        (
            ((self.hue as u16 + 180 - HALF_WIDTH as u16) % 180) as u8,
            ((self.hue as u16 + HALF_WIDTH as u16) % 180) as u8,
        )
    }
}

/// The two team colours of a video's kill-feed bars.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BarHues {
    /// The colour that replaces the default teal (the one closer to it).
    pub teal: TeamHue,
    /// The colour that replaces the default red.
    pub red: TeamHue,
}

impl BarHues {
    /// `base` with the hue ranges of the learned colours. The saturation
    /// and value minimums only go down (a filter that pales the bars), and
    /// the bar class's minimums follow them in proportion.
    #[must_use]
    pub fn params(&self, base: &ClassParams) -> ClassParams {
        let scaled = |bar: u8, class: u8, learned: u8| -> u8 {
            if learned >= class {
                bar
            } else {
                // `learned < class`, so the result is below `bar`.
                (u16::from(bar) * u16::from(learned) / u16::from(class.max(1))) as u8
            }
        };
        let (teal, red) = (self.teal, self.red);
        let (red_min, red_max) = red.range();
        ClassParams {
            teal_hue: teal.range(),
            teal_min_s: base.teal_min_s.min(teal.min_s),
            teal_min_v: base.teal_min_v.min(teal.min_v),
            red_hue_min: red_min,
            red_hue_max: red_max,
            red_min_s: base.red_min_s.min(red.min_s),
            red_min_v: base.red_min_v.min(red.min_v),
            bar_teal_min_v: scaled(base.bar_teal_min_v, base.teal_min_v, teal.min_v),
            bar_teal_min_s: scaled(base.bar_teal_min_s, base.teal_min_s, teal.min_s),
            bar_red_min_v: scaled(base.bar_red_min_v, base.red_min_v, red.min_v),
            ..*base
        }
    }
}

/// Learns a video's [`BarHues`] from the kill-feed rows of sampled frames.
///
/// A video keeps its two team colours, and over a video both teams die:
/// the colours next to the victim names gather in two clusters, which
/// become the detector's default hue ranges ([`BarHues::params`]). Videos
/// whose colours lie in the default ranges learn nothing.
#[derive(Debug, Clone, Default)]
pub struct BarHueLearner {
    tones: Vec<Tone>,
    rows: usize,
}

impl BarHueLearner {
    /// An empty learner.
    pub fn new() -> Self {
        Self::default()
    }

    /// Learns from the rows `detector` found in its last frame: the colour
    /// next to the victim name of every row found from colour (rows found
    /// from luminance alone carry no usable colour).
    pub fn push(&mut self, detector: &KillFeedDetector) {
        for row in detector.rows().iter().filter(|r| !r.is_low_confidence()) {
            if self.tones.len() >= MAX_SAMPLES {
                return;
            }
            self.rows += 1;
            if let Some(tone) = victim_tone(detector, *row) {
                self.tones.push(tone);
            }
        }
    }

    /// Rows learned from so far.
    pub const fn rows(&self) -> usize {
        self.rows
    }

    /// The learned team colours, or `None` while fewer than two clear
    /// colours were seen, or if both lie in the default ranges of `base`.
    pub fn hues(&self, base: &ClassParams) -> Option<BarHues> {
        let first = self.cluster(None)?;
        let second = self.cluster(Some(first.hue))?;
        let (teal, red) =
            if hue_distance(first.hue, TEAL_CENTRE) <= hue_distance(second.hue, TEAL_CENTRE) {
                (first, second)
            } else {
                (second, first)
            };
        let explained = in_hue_range(teal.hue, base.teal_hue)
            && in_hue_range(red.hue, (base.red_hue_min, base.red_hue_max));
        (!explained).then_some(BarHues { teal, red })
    }

    /// The largest cluster of victim colours (away from `other`, at least
    /// [`MIN_TEAM_HUE_GAP`] from it), if it has enough rows.
    fn cluster(&self, other: Option<u8>) -> Option<TeamHue> {
        let away = |h: u8| other.is_none_or(|o| hue_distance(h, o) >= MIN_TEAM_HUE_GAP);
        let mut hist = [0u32; 180];
        for t in self.tones.iter().filter(|t| away(t.hue())) {
            hist[usize::from(t.hue())] += 1;
        }
        let tol = usize::from(CLUSTER_TOLERANCE);
        let window =
            |h: usize| -> u32 { (0..=2 * tol).map(|k| hist[(h + 180 + k - tol) % 180]).sum() };
        let (mode, count) = (0..180).map(|h| (h, window(h))).max_by_key(|&(_, n)| n)?;
        let needed = (MIN_SHARE * self.rows as f32).ceil() as usize;
        if (count as usize) < MIN_ROWS.max(needed) {
            return None;
        }
        // The circular mean of the cluster's hues.
        let (sum, n) = (0..=2 * tol).fold((0i64, 0i64), |(s, n), k| {
            let c = i64::from(hist[(mode + 180 + k - tol) % 180]);
            (s + c * (k as i64 - tol as i64), n + c)
        });
        let hue = (mode as i64 + sum / n.max(1)).rem_euclid(180) as u8;
        if !away(hue) {
            return None;
        }
        let mut members: Vec<Tone> = self
            .tones
            .iter()
            .copied()
            .filter(|t| away(t.hue()) && hue_distance(t.hue(), hue) <= CLUSTER_TOLERANCE)
            .collect();
        if members.is_empty() {
            return None;
        }
        let median = |members: &mut Vec<Tone>, key: fn(&Tone) -> u8| {
            members.sort_by_key(key);
            key(&members[members.len() / 2])
        };
        Some(TeamHue {
            hue,
            min_s: median(&mut members, |t| t.min_s()),
            min_v: median(&mut members, |t| t.min_v()),
        })
    }
}

/// The bar colour next to the victim name of `row`, as the row analysis
/// learns it.
fn victim_tone(detector: &KillFeedDetector, row: FeedRow) -> Option<Tone> {
    tones::learn(
        &detector.classes,
        &Sample {
            rect: RectI {
                x0: VICTIM_COLOUR_X0,
                y0: row.top + INNER_MARGIN,
                x1: VICTIM_X1,
                y1: row.top + ROW_H - INNER_MARGIN,
            },
            exclude: None,
            min_share: VICTIM_TONE_SHARE,
            min_pixels: VICTIM_TONE_PIXELS,
            count_grey: true,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::{BarHueLearner, BarHues, TeamHue};
    use crate::classes::{Class, ClassParams};
    use crate::killfeed::tones::Tone;

    fn learner(hues: &[(u8, usize)]) -> BarHueLearner {
        let mut l = BarHueLearner::default();
        for &(hue, n) in hues {
            for i in 0..n {
                l.tones.push(Tone::new(hue + (i % 3) as u8, 70, 90));
                l.rows += 1;
            }
        }
        l
    }

    #[test]
    fn default_colours_learn_nothing() {
        let base = ClassParams::default();
        assert_eq!(learner(&[(80, 20), (176, 20)]).hues(&base), None);
    }

    #[test]
    fn two_clear_colours_replace_the_defaults() {
        let base = ClassParams::default();
        // A deuteranopia-like palette: blue and yellow.
        let hues = learner(&[(108, 20), (28, 14)]).hues(&base).unwrap();
        assert_eq!((hues.teal.hue, hues.red.hue), (109, 29));
        let p = hues.params(&base);
        assert_eq!(
            (p.teal_hue, (p.red_hue_min, p.red_hue_max)),
            ((94, 124), (14, 44))
        );
        let flags = |rgb| p.colour_flags(rgb);
        // A blue and a yellow bar pixel now carry the bar classes.
        assert_ne!(flags([40, 110, 220]) & Class::Teal.bit(), 0);
        assert_ne!(flags([220, 190, 40]) & Class::Red.bit(), 0);
        assert_eq!(flags([40, 200, 190]) & Class::Teal.bit(), 0);
    }

    #[test]
    fn one_colour_or_too_few_rows_is_not_enough() {
        let base = ClassParams::default();
        assert_eq!(learner(&[(108, 40)]).hues(&base), None);
        assert_eq!(learner(&[(108, 20), (28, 3)]).hues(&base), None);
        // The second colour is too close: one bar in two shades.
        assert_eq!(learner(&[(108, 20), (122, 20)]).hues(&base), None);
    }

    #[test]
    fn learned_minimums_only_lower_the_thresholds() {
        let base = ClassParams::default();
        let dim = TeamHue {
            hue: 120,
            min_s: 30,
            min_v: 35,
        };
        let bright = TeamHue {
            hue: 30,
            min_s: 200,
            min_v: 250,
        };
        let p = BarHues {
            teal: dim,
            red: bright,
        }
        .params(&base);
        assert_eq!((p.teal_min_s, p.teal_min_v), (30, 35));
        assert!(p.bar_teal_min_v < base.bar_teal_min_v && p.bar_teal_min_s < base.bar_teal_min_s);
        assert_eq!((p.red_min_s, p.red_min_v, p.bar_red_min_v), (85, 110, 150));
        // The red range wraps around 0 when the colour is near it.
        let near_zero = TeamHue { hue: 5, ..bright };
        assert_eq!(near_zero.range(), (170, 20));
    }
}
