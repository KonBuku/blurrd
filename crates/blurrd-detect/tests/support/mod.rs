//! Shared helpers of the detector tests: synthetic kill feeds, crops of
//! synthetic frames, colour filters like those of GPU drivers, and the
//! checks of a frame's detections against its ground truth.

#![allow(
    dead_code,
    reason = "each test binary uses a different subset of the helpers"
)]

use blurrd_blur::{PixelParams, PreparedBox, YuvColour, prepare_boxes};
use blurrd_core::{BlurBox, BlurParams, Region};
use blurrd_detect::killfeed::FEED_ROI;
use blurrd_detect::{Detection, KillFeedDetector, RgbView};
use blurrd_testkit::hud::killfeed::{Kill, KillFeedSpec, Team, Weapon};
use blurrd_testkit::metrics::item_coverage;
use blurrd_testkit::truth::ContentKind;
use blurrd_testkit::{
    Background, Fixture, FontSet, Layout, NameStyle, Palette, RgbFrame, Rng, Scene, synth_name,
};

/// Frames after its birth until a row has slid into place.
const SETTLED: u32 = 4;

/// A kill with every visual variant chosen from `rng`.
pub(crate) fn random_kill(rng: &mut Rng, birth: u32) -> Kill {
    let killer_style = *rng.pick(&NameStyle::LATIN_MIX);
    let victim_style = *rng.pick(&NameStyle::LATIN_MIX);
    let killer_team = *rng.pick(&[Team::Ally, Team::Enemy]);
    let victim_team = match (rng.chance(0.2), killer_team) {
        (true, team) => team,
        (false, Team::Ally) => Team::Enemy,
        (false, Team::Enemy) => Team::Ally,
    };
    Kill {
        killer: synth_name(rng, killer_style),
        victim: synth_name(rng, victim_style),
        killer_team,
        victim_team,
        weapon: *rng.pick(&[
            Weapon::Rifle,
            Weapon::Pistol,
            Weapon::Knife,
            Weapon::Ability,
        ]),
        badge: rng.chance(0.25).then(|| 2 + rng.below(4) as u8),
        own: rng.chance(0.25),
        headshot: rng.chance(0.5),
        birth,
    }
}

/// A frame of a scene with `rows` random rows born one frame apart, when
/// all of them have settled.
pub(crate) fn settled_fixture(
    seed: u64,
    rows: u32,
    background: Background,
    palette: Palette,
) -> Fixture {
    let fonts = FontSet::latin();
    let mut rng = Rng::new(seed);
    let kills = (0..rows).map(|i| random_kill(&mut rng, i)).collect();
    let scene = Scene::new(Layout::reference(), background, palette, rows + SETTLED + 2)
        .with_kill_feed(&fonts, KillFeedSpec::new(kills));
    scene.render(rows - 1 + SETTLED)
}

/// [`settled_fixture`], checked with a fresh detector.
pub(crate) fn settled_feed(
    seed: u64,
    rows: u32,
    background: Background,
    palette: Palette,
) -> Check {
    check_frame(&settled_fixture(seed, rows, background, palette))
}

/// A colour change applied to a whole frame, like the post-processing
/// filters of GPU drivers (NVIDIA Digital Vibrance and Freestyle, AMD
/// colour settings) or an edited video.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Filter {
    /// Rotates every hue by this many degrees (luminance-preserving
    /// matrix, as CSS `hue-rotate`).
    Hue(f32),
    /// Scales the saturation: 0 is grey, 1 unchanged, 2 doubled.
    Saturation(f32),
    /// Boosts the saturation of dull colours more than of vivid ones
    /// (Digital Vibrance); 0 is unchanged.
    Vibrance(f32),
    /// `255 · (c / 255)^gamma`.
    Gamma(f32),
    /// Adds `brightness · 255`, then scales the distance from mid-grey by
    /// `contrast`.
    Levels {
        /// Brightness offset (−1 to 1).
        brightness: f32,
        /// Contrast factor.
        contrast: f32,
    },
    /// Shifts red, green and blue separately in the shadows, midtones and
    /// highlights (−1 to 1 each), like a tint preset of NVIDIA Freestyle
    /// or FFmpeg's `colorbalance` filter, whose formula this follows.
    ColourBalance {
        /// Shifts of the dark tones.
        shadows: [f32; 3],
        /// Shifts of the middle tones.
        midtones: [f32; 3],
        /// Shifts of the bright tones.
        highlights: [f32; 3],
    },
}

impl Filter {
    /// The filtered colour of one pixel.
    fn apply(self, rgb: [u8; 3]) -> [u8; 3] {
        let c = rgb.map(f32::from);
        let luma = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        let out = match self {
            Self::Hue(degrees) => {
                let (sin, cos) = degrees.to_radians().sin_cos();
                let m = [
                    [
                        0.213 + cos * 0.787 - sin * 0.213,
                        0.715 - cos * 0.715 - sin * 0.715,
                        0.072 - cos * 0.072 + sin * 0.928,
                    ],
                    [
                        0.213 - cos * 0.213 + sin * 0.143,
                        0.715 + cos * 0.285 + sin * 0.140,
                        0.072 - cos * 0.072 - sin * 0.283,
                    ],
                    [
                        0.213 - cos * 0.213 - sin * 0.787,
                        0.715 - cos * 0.715 + sin * 0.715,
                        0.072 + cos * 0.928 + sin * 0.072,
                    ],
                ];
                m.map(|row| row[0] * c[0] + row[1] * c[1] + row[2] * c[2])
            }
            Self::Saturation(s) => c.map(|v| luma + s * (v - luma)),
            Self::Vibrance(amount) => {
                let (mx, mn) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
                let saturation = if mx > 0.0 { (mx - mn) / mx } else { 0.0 };
                let s = 1.0 + amount * (1.0 - saturation);
                c.map(|v| luma + s * (v - luma))
            }
            Self::Gamma(g) => c.map(|v| 255.0 * (v / 255.0).powf(g)),
            Self::Levels {
                brightness,
                contrast,
            } => c.map(|v| (v + brightness * 255.0 - 128.0) * contrast + 128.0),
            Self::ColourBalance {
                shadows,
                midtones,
                highlights,
            } => {
                // FFmpeg's weights over the sum of the largest and the
                // smallest channel (0 to 2).
                const SLOPE: f32 = 4.0;
                const SPLIT: f32 = 0.333;
                const SCALE: f32 = 0.7;
                let l = (c[0].max(c[1]).max(c[2]) + c[0].min(c[1]).min(c[2])) / 255.0;
                let weight = |v: f32| (v * SLOPE + 0.5).clamp(0.0, 1.0) * SCALE;
                let (ws, wh) = (weight(SPLIT - l), weight(l + SPLIT - 1.0));
                let wm = weight(l - SPLIT) * (weight(1.0 - l - SPLIT) / SCALE);
                std::array::from_fn(|i| {
                    c[i] + 255.0 * (ws * shadows[i] + wm * midtones[i] + wh * highlights[i])
                })
            }
        };
        out.map(|v| v.round().clamp(0.0, 255.0) as u8)
    }

    /// A copy of a reference-size frame with the kill-feed crop
    /// ([`FEED_ROI`], all the detector reads) filtered.
    pub(crate) fn frame(self, frame: &RgbFrame) -> RgbFrame {
        let mut data = frame.data().to_vec();
        let stride = 3 * frame.width() as usize;
        let (x0, x1) = (FEED_ROI.x0 as usize, FEED_ROI.x1 as usize);
        for y in FEED_ROI.y0 as usize..(FEED_ROI.y1 as usize).min(frame.height() as usize) {
            let row = &mut data[y * stride + 3 * x0..y * stride + 3 * x1];
            for pixel in row.as_chunks_mut::<3>().0 {
                *pixel = self.apply(*pixel);
            }
        }
        RgbFrame::from_raw(frame.width(), frame.height(), data).unwrap()
    }

    /// The fixture with its frame filtered (the truth does not change).
    pub(crate) fn fixture(self, fx: &Fixture) -> Fixture {
        Fixture {
            frame: self.frame(&fx.frame),
            truth: fx.truth.clone(),
        }
    }
}

/// The [`FEED_ROI`] of a reference-size frame, without copying.
pub(crate) fn feed_view(frame: &RgbFrame) -> RgbView<'_> {
    let stride = 3 * frame.width() as usize;
    let start = FEED_ROI.y0 as usize * stride + 3 * FEED_ROI.x0 as usize;
    RgbView::new(
        &frame.data()[start..],
        FEED_ROI.width(),
        FEED_ROI.height(),
        stride,
    )
    .unwrap()
}

/// The boxes the render would blur for `detections` with the default blur
/// parameters at reference scale.
pub(crate) fn prepared(frame: &RgbFrame, detections: &[Detection]) -> Vec<PreparedBox> {
    let params = PixelParams::new(&BlurParams::default(), 1.0, YuvColour::default()).unwrap();
    let boxes: Vec<BlurBox> = detections
        .iter()
        .map(|d| BlurBox {
            rect: d.rect,
            region: d.region,
            seed: 1,
            grid_origin: (d.text.x0, d.text.y0),
            block_scale: 1.0,
        })
        .collect();
    let mut out = vec![];
    prepare_boxes(&boxes, frame.width(), frame.height(), &params, &mut out);
    out
}

/// The kill-feed detections of one frame, checked against its truth.
#[derive(Debug, Clone, Default)]
pub(crate) struct Check {
    /// Kill-feed names in the truth.
    pub(crate) items: usize,
    /// Rows the detector found.
    pub(crate) rows: usize,
    /// Their tops.
    pub(crate) row_tops: Vec<i32>,
    /// What the detector reported.
    pub(crate) detections: Vec<Detection>,
    /// Names with glyph pixels the boxes do not fully cover.
    pub(crate) leaks: Vec<String>,
    /// Names no detection overlaps.
    pub(crate) missed: Vec<String>,
    /// Detections that overlap no kill-feed name and no multikill badge.
    pub(crate) false_positives: Vec<String>,
    /// Detections on a multikill badge: the roman numeral of a badge in
    /// front of a one-colour row passes as a name (as in the prototype).
    /// Over-blur only; the temporal model (M10) can drop them.
    pub(crate) badges: Vec<String>,
}

impl Check {
    /// Panics with the details if a name is missed or a detection is false.
    pub(crate) fn assert_found(&self, context: &str) {
        assert!(
            self.missed.is_empty() && self.false_positives.is_empty(),
            "{context}: missed {:?}, false positives {:?}, detections {:?}",
            self.missed,
            self.false_positives,
            self.detections
        );
    }

    /// Panics with the details if a name is missed, a detection is false or
    /// a glyph pixel stays readable.
    pub(crate) fn assert_clean(&self, context: &str) {
        self.assert_found(context);
        assert!(
            self.leaks.is_empty(),
            "{context}: leaks {:?}, detections {:?}",
            self.leaks,
            self.detections
        );
    }
}

/// Runs a fresh detector on a fixture and checks every kill-feed name.
pub(crate) fn check_frame(fx: &Fixture) -> Check {
    check_with(&mut KillFeedDetector::new(), fx)
}

/// Runs `detector` on a fixture and checks every kill-feed name.
pub(crate) fn check_with(detector: &mut KillFeedDetector, fx: &Fixture) -> Check {
    let mut detections = vec![];
    detector
        .detect(&feed_view(&fx.frame), &mut detections)
        .unwrap();
    let boxes = prepared(&fx.frame, &detections);
    let items: Vec<_> = fx.truth.in_region(Region::KillFeed).collect();
    let coverage = item_coverage(&fx.truth, &boxes);
    let leaks = coverage
        .iter()
        .filter(|c| c.region == Region::KillFeed && c.is_leak())
        .map(|c| {
            let item = items.iter().find(|i| i.id == c.id).unwrap();
            format!(
                "{:?} {:?} {} ({} of {} px)",
                item.id, item.kind, item.text, c.leaked_pixels, c.glyph_pixels
            )
        })
        .collect();
    // One-colour rows report both names as victims (port spec 3.3), so
    // only the region has to match.
    let on_badge = |d: &Detection| {
        fx.truth
            .content
            .iter()
            .any(|c| c.kind == ContentKind::Badge && c.rect.intersection(&d.text).is_some())
    };
    let (badges, false_positives): (Vec<_>, Vec<_>) = detections
        .iter()
        .filter(|d| !items.iter().any(|i| i.text.intersection(&d.text).is_some()))
        .partition(|d| on_badge(d));
    let describe = |d: &&Detection| format!("{:?} {}", d.kind, d.text);
    let (badges, false_positives) = (
        badges.iter().map(describe).collect(),
        false_positives.iter().map(describe).collect(),
    );
    let missed = items
        .iter()
        .filter(|i| {
            !detections
                .iter()
                .any(|d| i.text.intersection(&d.text).is_some())
        })
        .map(|i| format!("{:?} {:?} {}", i.id, i.kind, i.text))
        .collect();
    Check {
        items: items.len(),
        rows: detector.rows().len(),
        row_tops: detector.rows().iter().map(|r| r.top).collect(),
        detections,
        leaks,
        missed,
        false_positives,
        badges,
    }
}
