//! The health check (plan 7.3, M15): where a region's structure stays but
//! its detector reports no name for more than 30 frames, the span is
//! reported and covered with the structure's slot boxes. The detector
//! failure is simulated by dropping every detection of a span.

#![allow(
    clippy::unwrap_used,
    clippy::cast_possible_wrap,
    reason = "test helpers on small frames: a panic is a failure"
)]

use std::ops::Range;

use blurrd_blur::{PixelParams, PreparedBox, YuvColour, prepare_boxes};
use blurrd_core::{BlurBox, BlurParams, FrameRange, RectI, Region};
use blurrd_detect::chat::LEFT_PANEL_ROI;
use blurrd_detect::health::{HEALTH_FRAMES, HealthCheck};
use blurrd_detect::killfeed::FEED_ROI;
use blurrd_detect::{ChatDetector, KillFeedDetector, RgbView, VoiceDetector};
use blurrd_testkit::hud::chat::{Channel, ChatMessage, ChatSpec, Language};
use blurrd_testkit::hud::killfeed::{Kill, KillFeedSpec, Team, Weapon};
use blurrd_testkit::hud::voice::{self, Speaker, VoiceSpec};
use blurrd_testkit::metrics::item_coverage;
use blurrd_testkit::{
    Background, Fixture, FontSet, Layout, NameStyle, Palette, RgbFrame, Rng, Scene, synth_name,
};

/// Frames of the scene.
const FRAMES: u32 = 90;
/// Every name has arrived and settled from this frame on.
const SETTLED: u32 = 12;

/// A scene with kills, chat messages and voice speakers that stay on
/// screen for the whole scene.
fn scene() -> Scene {
    let fonts = FontSet::latin();
    let mut rng = Rng::new(7);
    let kills = (0..3u32)
        .map(|i| Kill {
            killer: synth_name(&mut rng, NameStyle::Latin(6 + 2 * i as usize)),
            victim: synth_name(&mut rng, NameStyle::Latin(12 - 2 * i as usize)),
            killer_team: Team::Ally,
            victim_team: Team::Enemy,
            weapon: Weapon::Rifle,
            badge: None,
            own: false,
            headshot: false,
            birth: 2 * i,
        })
        .collect();
    let messages = [Channel::All, Channel::Team]
        .into_iter()
        .zip(0u32..)
        .map(|(channel, i)| ChatMessage {
            channel,
            name: synth_name(&mut rng, NameStyle::Latin(7 + 2 * i as usize)),
            team: Team::Ally,
            text: "push long".to_owned(),
            role: None,
            wrap: false,
            time: 1 + 2 * i,
        })
        .collect();
    let speakers = (0..2u8)
        .map(|slot| Speaker {
            name: synth_name(&mut rng, NameStyle::Latin(8)),
            location: voice::LOCATIONS[usize::from(slot)].to_owned(),
            slot,
            shown: vec![FrameRange {
                f0: 0,
                f1: FRAMES - 1,
            }],
        })
        .collect();
    Scene::new(
        Layout::reference(),
        Background::dusk(),
        Palette::DEFAULT,
        FRAMES,
    )
    .with_kill_feed(&fonts, KillFeedSpec::new(kills))
    .with_chat(&fonts, ChatSpec::new(Language::English, messages))
    .with_voice(&fonts, VoiceSpec { speakers })
}

/// The reference rectangle `roi` of a reference-size frame.
fn crop(frame: &RgbFrame, roi: RectI) -> RgbView<'_> {
    let stride = 3 * frame.width() as usize;
    let start = roi.y0 as usize * stride + 3 * roi.x0 as usize;
    RgbView::new(&frame.data()[start..], roi.width(), roi.height(), stride).unwrap()
}

/// The detectors of the three regions.
struct Detectors {
    feed: KillFeedDetector,
    chat: ChatDetector,
    voice: VoiceDetector,
}

impl Detectors {
    /// The structure slots of `region` in `fx` and whether its detector
    /// found a name.
    fn read(&mut self, fx: &Fixture, region: Region) -> (Vec<RectI>, bool) {
        let mut dets = Vec::new();
        let mut slots = Vec::new();
        match region {
            Region::KillFeed => {
                self.feed
                    .detect(&crop(&fx.frame, FEED_ROI), &mut dets)
                    .unwrap();
                self.feed.row_slots(&mut slots);
            }
            Region::TextChat => {
                self.chat
                    .detect(&crop(&fx.frame, LEFT_PANEL_ROI), &mut dets)
                    .unwrap();
                self.chat.band_slots(&mut slots);
            }
            _ => {
                self.voice
                    .detect(&crop(&fx.frame, LEFT_PANEL_ROI), &mut dets)
                    .unwrap();
                let shown = self.voice.slots().iter().filter(|s| s.shows_entry());
                slots.extend(shown.map(blurrd_detect::voice::SlotEvidence::slot_box));
            }
        }
        (slots, !dets.is_empty())
    }
}

/// The default blur's boxes for `rects` in a reference-size frame.
fn prepared(frame: &RgbFrame, rects: &[RectI], region: Region) -> Vec<PreparedBox> {
    let params = PixelParams::new(&BlurParams::default(), 1.0, YuvColour::default()).unwrap();
    let boxes: Vec<BlurBox> = rects
        .iter()
        .map(|r| BlurBox {
            rect: *r,
            region,
            seed: 1,
            grid_origin: (r.x0, r.y0),
            block_scale: 1.0,
        })
        .collect();
    let mut out = vec![];
    prepare_boxes(&boxes, frame.width(), frame.height(), &params, &mut out);
    out
}

/// A health check run: the check, the slot boxes per frame and the frames.
type Run = (HealthCheck, Vec<(u32, Vec<RectI>)>, Vec<Fixture>);

/// Runs `region` of the scene through a health check with every detection
/// of the frames in `lost` dropped. Returns the check and the slot boxes
/// per frame.
fn run(region: Region, lost: Range<u32>) -> Run {
    let scene = scene();
    let mut detectors = Detectors {
        feed: KillFeedDetector::new(),
        chat: ChatDetector::new(),
        voice: VoiceDetector::new(),
    };
    let mut check = HealthCheck::new(region);
    let mut out = Vec::new();
    let mut fixtures = Vec::new();
    for f in SETTLED..FRAMES {
        let fx = scene.render(f);
        let (slots, names) = detectors.read(&fx, region);
        assert!(!slots.is_empty(), "{region:?} frame {f}: no structure");
        assert!(names, "{region:?} frame {f}: the detector found no name");
        check.push(f, &slots, names && !lost.contains(&f));
        check.drain_ready(&mut out);
        fixtures.push(fx);
    }
    check.finish(&mut out);
    (check, out, fixtures)
}

#[test]
fn a_missing_detector_span_is_reported_and_covered_with_slot_boxes() {
    let lost = 30..70;
    for region in [Region::KillFeed, Region::TextChat, Region::VoiceChat] {
        let (check, out, fixtures) = run(region, lost.clone());
        let spans: Vec<FrameRange> = check.spans().iter().map(|s| s.frames).collect();
        assert_eq!(spans, [FrameRange { f0: 30, f1: 69 }], "{region:?}");
        assert!(check.spans().iter().all(|s| s.region == region));
        for ((f, slots), fx) in out.iter().zip(&fixtures) {
            assert_eq!(!slots.is_empty(), lost.contains(f), "{region:?} frame {f}");
            if slots.is_empty() {
                continue;
            }
            let boxes = prepared(&fx.frame, slots, region);
            let names = fx.truth.in_region(region).count();
            assert!(names >= 2, "{region:?} frame {f}: {names} names");
            let leaks: Vec<_> = item_coverage(&fx.truth, &boxes)
                .into_iter()
                .filter(|c| c.region == region && c.is_leak())
                .collect();
            assert!(leaks.is_empty(), "{region:?} frame {f}: {leaks:?}");
        }
    }
}

#[test]
fn a_short_detector_gap_is_left_to_the_temporal_models() {
    let lost = 30..30 + HEALTH_FRAMES;
    for region in [Region::KillFeed, Region::TextChat, Region::VoiceChat] {
        let (check, out, _) = run(region, lost.clone());
        assert_eq!(check.spans(), &[], "{region:?}");
        assert!(out.iter().all(|(_, s)| s.is_empty()), "{region:?}");
    }
}
