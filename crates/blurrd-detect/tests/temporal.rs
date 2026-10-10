//! The kill-feed tracker on synthetic videos: every name stays covered
//! while rows slide in, jump up the stack and are washed out by flashes,
//! also on colour-blind palettes, under GPU colour filters and on a
//! background that changes every frame.

#![allow(
    clippy::unwrap_used,
    reason = "test helpers on small frames: a panic is a failure"
)]

mod support;

use std::collections::{BTreeMap, BTreeSet};

use blurrd_blur::{PixelParams, PreparedBox, YuvColour, prepare_boxes};
use blurrd_core::{BlurBox, BlurParams, Confidence, NameKind, RectI, Region};
use blurrd_detect::KillFeedDetector;
use blurrd_detect::temporal::killfeed::{
    DELAY, FrameBoxes, KillFeedFrame, KillFeedTracker, POST_ROLL, PRE_ROLL, RING_FRAMES, TrackError,
};
use blurrd_testkit::hud::killfeed::{Kill, KillFeedSpec, Team};
use blurrd_testkit::metrics::item_coverage;
use blurrd_testkit::truth::ContentKind;
use blurrd_testkit::{
    Background, Fixture, Flash, FontSet, FrameTruth, ItemId, Layout, Palette, Rng, Scene,
};

use support::{Filter, feed_view, random_kill};

/// A synthetic video: one scene per background variant, the variant
/// changing every frame (a background that moves like gameplay does).
struct Video {
    scenes: Vec<Scene>,
    filter: Option<Filter>,
}

impl Video {
    fn new(
        seed: u64,
        births: &[u32],
        spec: impl Fn(KillFeedSpec) -> KillFeedSpec,
        backgrounds: &[Background],
        palette: Palette,
        frames: u32,
    ) -> Self {
        let fonts = FontSet::latin();
        let mut rng = Rng::new(seed);
        let kills: Vec<_> = births.iter().map(|&b| random_kill(&mut rng, b)).collect();
        let scenes = backgrounds
            .iter()
            .map(|bg| {
                Scene::new(Layout::reference(), *bg, palette, frames)
                    .with_kill_feed(&fonts, spec(KillFeedSpec::new(kills.clone())))
            })
            .collect();
        Self {
            scenes,
            filter: None,
        }
    }

    fn frames(&self) -> u32 {
        self.scenes[0].frames()
    }

    fn render(&self, frame: u32) -> Fixture {
        let fx = self.scenes[frame as usize % self.scenes.len()].render(frame);
        match self.filter {
            Some(f) => f.fixture(&fx),
            None => fx,
        }
    }

    fn with_flash(mut self, flash: Flash) -> Self {
        self.scenes = self
            .scenes
            .into_iter()
            .map(|s| s.with_flash(flash))
            .collect();
        self
    }
}

/// The result of running the detector and the tracker over a video.
struct Run {
    /// The tracker's boxes of every frame.
    boxes: Vec<FrameBoxes>,
    /// The truth of every frame.
    truths: Vec<FrameTruth>,
    /// Names the per-frame detector leaves readable in most frames where
    /// they are settled and opaque: the tracker follows what the detector
    /// finds and cannot cover the rest either.
    detector_limits: BTreeSet<ItemId>,
}

/// Runs the detector and the tracker over the whole video.
fn track(video: &Video) -> Run {
    let mut detector = KillFeedDetector::new();
    let mut tracker = KillFeedTracker::new();
    let mut out = Vec::new();
    let mut truths = Vec::new();
    let mut detections = Vec::new();
    let mut per_frame: BTreeMap<ItemId, (u32, u32)> = BTreeMap::new();
    for f in 0..video.frames() {
        let fx = video.render(f);
        let crop = feed_view(&fx.frame);
        detector.detect(&crop, &mut detections).unwrap();
        for c in item_coverage(&fx.truth, &support::prepared(&fx.frame, &detections)) {
            let item = fx.truth.items.iter().find(|i| i.id == c.id).unwrap();
            if item.region == Region::KillFeed && item.opacity >= 1.0 {
                let e = per_frame.entry(c.id).or_default();
                e.0 += 1;
                e.1 += u32::from(c.is_leak());
            }
        }
        truths.push(fx.truth);
        tracker
            .push(&KillFeedFrame {
                frame: f,
                crop,
                detections: &detections,
            })
            .unwrap();
        tracker.drain_ready(&mut out);
    }
    tracker.finish(&mut out);
    assert_eq!(out.len(), video.frames() as usize, "one entry per frame");
    for (f, entry) in (0..).zip(&out) {
        assert_eq!(entry.frame, f, "frames in order");
    }
    // Settled frames are all but the first few (the slide-in).
    let detector_limits = per_frame
        .into_iter()
        .filter(|&(_, (frames, leaked))| frames > 8 && 2 * leaked > frames)
        .map(|(id, _)| id)
        .collect();
    Run {
        boxes: out,
        truths,
        detector_limits,
    }
}

/// The boxes the render applies with the default blur parameters.
fn prepared(boxes: &FrameBoxes) -> Vec<PreparedBox> {
    let params = PixelParams::new(&BlurParams::default(), 1.0, YuvColour::default()).unwrap();
    let blur: Vec<BlurBox> = boxes
        .boxes
        .iter()
        .map(|b| BlurBox {
            rect: b.rect,
            region: Region::KillFeed,
            seed: 1,
            grid_origin: (b.rect.x0, b.rect.y0),
            block_scale: 1.0,
        })
        .collect();
    let mut out = Vec::new();
    prepare_boxes(&blur, 1920, 1080, &params, &mut out);
    out
}

/// Name-frames with a readable glyph pixel, among the names at least
/// `min_opacity` opaque and not beyond the per-frame detector, as
/// `frame: id kind` strings.
fn leaks(run: &Run, min_opacity: f32) -> (usize, Vec<String>) {
    let mut checked = 0;
    let mut leaks = Vec::new();
    for (entry, truth) in run.boxes.iter().zip(&run.truths) {
        let prepared = prepared(entry);
        for c in item_coverage(truth, &prepared) {
            let item = truth.items.iter().find(|i| i.id == c.id).unwrap();
            if item.region != Region::KillFeed
                || item.opacity < min_opacity
                || run.detector_limits.contains(&item.id)
            {
                continue;
            }
            checked += 1;
            if c.is_leak() {
                leaks.push(format!(
                    "{}: {:?} {:?} {} opacity {:.2} ({} of {} px)",
                    entry.frame,
                    item.id,
                    item.kind,
                    item.text,
                    item.opacity,
                    c.leaked_pixels,
                    c.glyph_pixels
                ));
            }
        }
    }
    (checked, leaks)
}

/// Boxes that overlap no kill-feed name of their frame or of the
/// `window` frames before it (pre-roll boxes: of the [`PRE_ROLL`] frames
/// after it), and boxes on a multikill badge.
fn strays(run: &Run, window: u32) -> Vec<String> {
    let mut out = Vec::new();
    for (entry, truth) in run.boxes.iter().zip(&run.truths) {
        let f = entry.frame as usize;
        for b in &entry.boxes {
            let recent = if b.confidence == Confidence::PreRoll {
                &run.truths[f..(f + 1 + PRE_ROLL as usize).min(run.truths.len())]
            } else {
                &run.truths[f.saturating_sub(window as usize)..=f]
            };
            let on_name = recent.iter().any(|t| {
                t.in_region(Region::KillFeed)
                    .any(|i| i.text.intersection(&b.rect).is_some())
            });
            let on_badge = truth
                .content
                .iter()
                .any(|c| c.kind == ContentKind::Badge && c.rect.intersection(&b.rect).is_some());
            if !on_name || on_badge {
                out.push(format!(
                    "{}: {:?} {} {:?} badge {on_badge}",
                    entry.frame, b.kind, b.rect, b.confidence
                ));
            }
        }
    }
    out
}

/// Rows born close together (wipe-in of every row, stacks of up to six),
/// living 50 frames: the oldest fade out and the rest jump up.
const BIRTHS: [u32; 9] = [0, 4, 9, 12, 30, 34, 70, 74, 78];

fn short_life(spec: KillFeedSpec) -> KillFeedSpec {
    KillFeedSpec { life: 50, ..spec }
}

#[test]
fn covers_names_through_wipe_in_and_stack_jumps() {
    let video = Video::new(
        11,
        &BIRTHS,
        short_life,
        &[Background::noise(5)],
        Palette::DEFAULT,
        160,
    );
    let run = track(&video);
    assert!(run.detector_limits.is_empty(), "{:?}", run.detector_limits);
    let (checked, leaks) = leaks(&run, 1.0);
    assert!(checked > 600, "{checked} name-frames");
    assert!(leaks.is_empty(), "{} leaks: {leaks:#?}", leaks.len());
    let strays = strays(&run, POST_ROLL);
    assert!(strays.is_empty(), "{strays:#?}");
}

#[test]
fn covers_fading_names_until_their_rows_are_gone() {
    // Long fades (the default 18 frames) of rows that are not replaced:
    // the template loses a name long before it is invisible, the row's
    // structure keeps the box.
    let video = Video::new(
        17,
        &[0, 6, 40],
        |s| KillFeedSpec { life: 60, ..s },
        &[Background::noise(9), Background::dusk()],
        Palette::DEFAULT,
        150,
    );
    let run = track(&video);
    let (checked, leaks) = leaks(&run, 0.0);
    assert!(checked > 300, "{checked} name-frames");
    assert!(leaks.is_empty(), "{} leaks: {leaks:#?}", leaks.len());
    let strays = strays(&run, POST_ROLL);
    assert!(strays.is_empty(), "{strays:#?}");
}

#[test]
fn keeps_companions_in_their_row_when_a_name_repeats() {
    // The same victim in two rows, the newer a one-colour row: when the
    // older row leaves, the victim's template of the older row continues
    // on the identical name of the newer row, and must not bring the
    // older row's killer along onto the newer row's killer portrait.
    let fonts = FontSet::latin();
    let mut rng = Rng::new(18);
    let kill = |killer: &str, victim: &str, one_colour: bool, birth: u32, rng: &mut Rng| Kill {
        killer: killer.to_owned(),
        victim: victim.to_owned(),
        killer_team: Team::Enemy,
        victim_team: if one_colour { Team::Enemy } else { Team::Ally },
        badge: None,
        ..random_kill(rng, birth)
    };
    let kills = vec![
        kill("Brightwater", "Corvane", false, 0, &mut rng),
        kill("Tallowmere77", "Corvane", true, 20, &mut rng),
    ];
    let video = Video {
        scenes: [Background::noise(3), Background::dusk()]
            .into_iter()
            .map(|bg| {
                Scene::new(Layout::reference(), bg, Palette::DEFAULT, 150).with_kill_feed(
                    &fonts,
                    KillFeedSpec {
                        life: 70,
                        ..KillFeedSpec::new(kills.clone())
                    },
                )
            })
            .collect(),
        filter: None,
    };
    let run = track(&video);
    let (checked, leaks) = leaks(&run, 1.0);
    assert!(checked > 150, "{checked} name-frames");
    assert!(leaks.is_empty(), "{} leaks: {leaks:#?}", leaks.len());
    let strays = strays(&run, POST_ROLL);
    assert!(strays.is_empty(), "{strays:#?}");
    let on_portraits: Vec<String> = run
        .boxes
        .iter()
        .zip(&run.truths)
        .flat_map(|(entry, truth)| {
            entry
                .boxes
                .iter()
                .filter(|b| truth.portraits().any(|p| p.intersection(&b.rect).is_some()))
                .map(|b| format!("{}: {:?} {}", entry.frame, b.kind, b.rect))
                .collect::<Vec<_>>()
        })
        .collect();
    assert!(on_portraits.is_empty(), "{on_portraits:#?}");
}

#[test]
fn grows_companions_read_short_while_the_row_wipes_in() {
    // In the first frames of a row the detector reads the killer without
    // its last glyphs (here they are painted over with the bar), and the
    // victim's reference frame is one of them. The killer's own reading
    // starts at the same column and is longer: the victim part's
    // companion box must grow to it, also in those first frames.
    let fonts = FontSet::latin();
    let mut rng = Rng::new(19);
    let kill = Kill {
        killer: "Tallowmere77".to_owned(),
        victim: "Corvane".to_owned(),
        killer_team: Team::Enemy,
        victim_team: Team::Ally,
        badge: None,
        ..random_kill(&mut rng, 0)
    };
    let scene = Scene::new(
        Layout::reference(),
        Background::dusk(),
        Palette::DEFAULT,
        90,
    )
    .with_kill_feed(
        &fonts,
        KillFeedSpec {
            life: 60,
            ..KillFeedSpec::new(vec![kill])
        },
    );
    let mut detector = KillFeedDetector::new();
    let mut tracker = KillFeedTracker::new();
    let (mut detections, mut out, mut truths) = (Vec::new(), Vec::new(), Vec::new());
    let mut short = 0;
    for f in 0..scene.frames() {
        let mut fx = scene.render(f);
        detector
            .detect(&feed_view(&fx.frame), &mut detections)
            .unwrap();
        let killer = fx
            .truth
            .in_region(Region::KillFeed)
            .find(|i| i.kind == NameKind::Killer)
            .map(|i| i.text);
        let read = |dets: &[blurrd_detect::Detection], text: RectI| {
            dets.iter()
                .find(|d| d.kind == NameKind::Killer && d.text.intersection(&text).is_some())
                .map(|d| d.text)
        };
        if let Some(text) = killer.filter(|&t| short < 2 && read(&detections, t).is_some()) {
            let cut = i32::midpoint(text.x0, text.x1);
            for y in text.y0 - 1..=text.y1 + 1 {
                if let Some(bar) = fx.frame.pixel(text.x1 + 4, y) {
                    for x in cut..=text.x1 + 1 {
                        fx.frame.set(x, y, bar);
                    }
                }
            }
            detector
                .detect(&feed_view(&fx.frame), &mut detections)
                .unwrap();
            let shortened = read(&detections, text).unwrap();
            assert!(
                shortened.x1 <= cut,
                "{f}: read {shortened} after the cut at {cut}"
            );
            short += 1;
        }
        tracker
            .push(&KillFeedFrame {
                frame: f,
                crop: feed_view(&fx.frame),
                detections: &detections,
            })
            .unwrap();
        tracker.drain_ready(&mut out);
        truths.push(fx.truth);
    }
    tracker.finish(&mut out);
    assert_eq!(short, 2, "the killer was read short twice");
    // The truth keeps the whole name: no box on it may end inside it (a
    // slot box of the sliding row hides such a box here; on real video
    // the frame can be a confident one), and nothing leaks.
    let mut short_boxes = Vec::new();
    let mut leaks = Vec::new();
    for (entry, truth) in out.iter().zip(&truths) {
        for item in truth.in_region(Region::KillFeed) {
            short_boxes.extend(
                entry
                    .boxes
                    .iter()
                    .filter(|b| b.rect.intersection(&item.text).is_some())
                    .filter(|b| b.rect.x1 < item.text.x1)
                    .map(|b| {
                        format!(
                            "{}: {} on {:?} {}",
                            entry.frame, b.rect, item.kind, item.text
                        )
                    }),
            );
        }
        for c in item_coverage(truth, &prepared(entry)) {
            let item = truth.items.iter().find(|i| i.id == c.id).unwrap();
            if item.opacity >= 1.0 && c.is_leak() {
                leaks.push(format!("{}: {:?} {}", entry.frame, item.kind, item.text));
            }
        }
    }
    assert!(short_boxes.is_empty(), "{short_boxes:#?}");
    assert!(leaks.is_empty(), "{leaks:#?}");
}

#[test]
fn drops_multikill_badges_of_one_colour_rows() {
    let fonts = FontSet::latin();
    let mut rng = Rng::new(16);
    let kills = (0..4)
        .map(|i| Kill {
            victim_team: Team::Enemy,
            killer_team: Team::Enemy,
            badge: Some(2 + i as u8),
            ..random_kill(&mut rng, 3 * i)
        })
        .collect();
    let video = Video {
        scenes: vec![
            Scene::new(
                Layout::reference(),
                Background::noise(4),
                Palette::DEFAULT,
                90,
            )
            .with_kill_feed(&fonts, short_life(KillFeedSpec::new(kills))),
        ],
        filter: None,
    };
    let run = track(&video);
    let (_, leaks) = leaks(&run, 1.0);
    assert!(leaks.is_empty(), "{leaks:#?}");
    let strays = strays(&run, POST_ROLL);
    assert!(strays.is_empty(), "{strays:#?}");
}

#[test]
fn covers_names_on_a_moving_background() {
    let video = Video::new(
        12,
        &BIRTHS,
        short_life,
        &Background::variety(9),
        Palette::DEFAULT,
        160,
    );
    let run = track(&video);
    let (_, leaks) = leaks(&run, 1.0);
    assert!(leaks.is_empty(), "{} leaks: {leaks:#?}", leaks.len());
}

#[test]
fn holds_names_through_a_flash() {
    // A flash washes the feed out almost completely for its first frames.
    let video = Video::new(
        13,
        &[0, 5, 10],
        short_life,
        &[Background::noise(6)],
        Palette::DEFAULT,
        100,
    )
    .with_flash(Flash {
        start: 25,
        frames: 12,
        amount: 0.9,
        area: None,
    });
    let run = track(&video);
    let (_, leaks) = leaks(&run, 1.0);
    assert!(leaks.is_empty(), "{} leaks: {leaks:#?}", leaks.len());
}

#[test]
fn covers_names_with_colour_blind_palettes_and_filters() {
    let filters = [
        None,
        Some(Filter::Hue(120.0)),
        Some(Filter::Saturation(0.0)),
        Some(Filter::Levels {
            brightness: -0.12,
            contrast: 0.85,
        }),
    ];
    for (i, palette) in Palette::ALL.iter().enumerate() {
        let mut video = Video::new(
            20 + i as u64,
            &BIRTHS[..6],
            short_life,
            &[Background::noise(i as u64), Background::dusk()],
            *palette,
            110,
        );
        video.filter = filters[i];
        let run = track(&video);
        let (_, leaks) = leaks(&run, 1.0);
        let context = format!("{} / {:?}", palette.name, filters[i]);
        assert!(
            leaks.is_empty(),
            "{context}: {} leaks: {leaks:#?}",
            leaks.len()
        );
        // In black and white the per-frame detector loses words of names
        // on light bars next to a textured portrait (two of this video's
        // names, see the M10 decisions in the plan).
        assert!(
            run.detector_limits.len() <= 2,
            "{context}: per-frame misses {:?}",
            run.detector_limits
        );
    }
}

#[test]
fn hands_out_frames_after_the_delay_and_bounds_its_memory() {
    let video = Video::new(
        14,
        &[0, 40, 80, 120, 160, 200, 240, 280, 320, 360],
        |s| KillFeedSpec { life: 70, ..s },
        &[Background::noise(8)],
        Palette::DEFAULT,
        420,
    );
    let mut detector = KillFeedDetector::new();
    let mut tracker = KillFeedTracker::new();
    let (mut out, mut detections) = (Vec::new(), Vec::new());
    let mut peak_late = 0;
    let mut filled = 0;
    let mut handed_out = 0;
    for f in 0..video.frames() {
        let fx = video.render(f);
        let crop = feed_view(&fx.frame);
        detector.detect(&crop, &mut detections).unwrap();
        let input = KillFeedFrame {
            frame: f,
            crop,
            detections: &detections,
        };
        tracker.push(&input).unwrap();
        tracker.drain_ready(&mut out);
        handed_out += out.len() as u32;
        out.clear();
        // Frame `f − DELAY` is the newest frame handed out.
        assert_eq!(handed_out, (f + 1).saturating_sub(DELAY));
        if f == 2 * RING_FRAMES {
            filled = tracker.heap_bytes();
        }
        if f > 2 * RING_FRAMES {
            peak_late = peak_late.max(tracker.heap_bytes());
        }
    }
    // Once the ring is full the memory stays flat (names come and go).
    assert!(
        peak_late <= filled + filled / 10,
        "{peak_late} B late vs {filled} B"
    );
    // Plan acceptance: at most 64 MB on any length of video.
    assert!(filled < 64 << 20, "{filled} B");
}

#[test]
fn rejects_frames_out_of_order_and_wrong_crops() {
    let video = Video::new(
        15,
        &[0],
        |s| s,
        &[Background::noise(1)],
        Palette::DEFAULT,
        3,
    );
    let fx = video.render(0);
    let crop = feed_view(&fx.frame);
    let mut tracker = KillFeedTracker::new();
    let frame = |frame| KillFeedFrame {
        frame,
        crop,
        detections: &[],
    };
    tracker.push(&frame(5)).unwrap();
    assert_eq!(
        tracker.push(&frame(7)),
        Err(TrackError::FrameOrder {
            expected: 6,
            actual: 7
        })
    );
    let small = blurrd_detect::RgbView::new(fx.frame.data(), 10, 10, 30).unwrap();
    assert!(matches!(
        tracker.push(&KillFeedFrame {
            crop: small,
            ..frame(6)
        }),
        Err(TrackError::CropSize { .. })
    ));
    let mut out = Vec::new();
    tracker.finish(&mut out);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].boxes.len(), 0);
}
