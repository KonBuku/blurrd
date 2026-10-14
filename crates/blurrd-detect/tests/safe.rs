//! Safe mode (plan 7.3, M15): every name gets the union of its tight box
//! and its slot box. On a synthetic scene with a kill feed, a text chat,
//! voice-chat speakers and an ACE banner, run through the per-frame
//! detectors and the temporal models, Safe mode leaks no name in any
//! region and blurs at most 0.6 of its pixels away from the names (port
//! spec 11.3 over-blur, over all regions; the slots of short names in the
//! voice chat and the banner's fallback width blur more on their own).

#![allow(
    clippy::unwrap_used,
    clippy::cast_possible_wrap,
    reason = "test helpers on small frames: a panic is a failure"
)]

mod support;

use blurrd_core::{FrameRange, Mode, RectI, Region};
use blurrd_detect::banner::BannerBox;
use blurrd_detect::chat::LEFT_PANEL_ROI;
use blurrd_detect::temporal::killfeed::{FrameBoxes, KillFeedFrame};
use blurrd_detect::voice::{VoiceFrame, VoiceFrameBoxes};
use blurrd_detect::{
    BannerDetector, BannerTracker, ChatDetector, ChatTracker, KillFeedDetector, KillFeedTracker,
    RgbView, VoiceDetector, VoiceTracker,
};
use blurrd_testkit::hud::banner::BannerSpec;
use blurrd_testkit::hud::chat::{Channel, ChatMessage, ChatSpec, Language};
use blurrd_testkit::hud::killfeed::{KillFeedSpec, Team};
use blurrd_testkit::hud::voice::{self, Speaker, VoiceSpec};
use blurrd_testkit::metrics::{OverBlur, item_coverage, over_blur};
use blurrd_testkit::{
    Background, Fixture, FontSet, Layout, NameStyle, Palette, RgbFrame, Rng, Scene, synth_name,
};
use support::{feed_view, prepared_rects, random_kill, render_all};

/// The highest over-blur ratio Safe mode may have (plan 15, M15).
const MAX_SAFE_OVER_BLUR: f64 = 0.6;

/// The scene: six kills, three chat messages, two speakers and an ACE
/// banner, all on screen together for a while.
fn scene() -> Scene {
    let fonts = FontSet::latin();
    let mut rng = Rng::new(11);
    let kills = (0..6).map(|i| random_kill(&mut rng, 3 * i)).collect();
    let messages = [Channel::All, Channel::Team, Channel::Broadcast]
        .into_iter()
        .zip(0u32..)
        .map(|(channel, i)| ChatMessage {
            channel,
            name: synth_name(&mut rng, NameStyle::Latin(6 + 4 * i as usize)),
            team: Team::Ally,
            text: "rotate to a".to_owned(),
            role: (channel == Channel::Broadcast).then(|| "Sova".to_owned()),
            wrap: false,
            time: 2 + 4 * i,
        })
        .collect();
    let banner = BannerSpec {
        hold: 60,
        ..BannerSpec::new("ACE", &synth_name(&mut rng, NameStyle::Latin(9)), 20)
    };
    let frames = banner.end() + 20;
    let speakers = (0..2u8)
        .map(|slot| Speaker {
            name: synth_name(&mut rng, NameStyle::Latin(5 + 5 * usize::from(slot))),
            location: voice::LOCATIONS[usize::from(slot)].to_owned(),
            slot,
            shown: vec![FrameRange {
                f0: 4,
                f1: frames - 10,
            }],
        })
        .collect();
    Scene::new(
        Layout::reference(),
        Background::dusk(),
        Palette::DEFAULT,
        frames,
    )
    .with_kill_feed(&fonts, KillFeedSpec::new(kills))
    .with_chat(&fonts, ChatSpec::new(Language::English, messages))
    .with_voice(&fonts, VoiceSpec { speakers })
    .with_banner(&fonts, banner)
}

/// The left panel of a reference-size frame.
fn left_view(frame: &RgbFrame) -> RgbView<'_> {
    let stride = 3 * frame.width() as usize;
    let start = LEFT_PANEL_ROI.y0 as usize * stride + 3 * LEFT_PANEL_ROI.x0 as usize;
    RgbView::new(
        &frame.data()[start..],
        LEFT_PANEL_ROI.width(),
        LEFT_PANEL_ROI.height(),
        stride,
    )
    .unwrap()
}

/// The boxes of every frame for `region`, with the temporal model in
/// `mode`.
fn boxes(frames: &[Fixture], region: Region, mode: Mode) -> Vec<Vec<RectI>> {
    let mut per_frame = vec![Vec::new(); frames.len()];
    let mut dets = Vec::new();
    match region {
        Region::KillFeed => {
            let (mut detector, mut tracker) = (KillFeedDetector::new(), KillFeedTracker::new());
            tracker.set_mode(mode);
            let mut out: Vec<FrameBoxes> = Vec::new();
            for (f, fx) in (0u32..).zip(frames) {
                let crop = feed_view(&fx.frame);
                detector.detect(&crop, &mut dets).unwrap();
                let frame = KillFeedFrame {
                    frame: f,
                    crop,
                    detections: &dets,
                };
                tracker.push(&frame).unwrap();
                tracker.drain_ready(&mut out);
            }
            tracker.finish(&mut out);
            for b in out {
                per_frame[b.frame as usize].extend(b.boxes.iter().map(|b| b.rect));
            }
        }
        Region::TextChat => {
            let (mut detector, mut tracker) = (ChatDetector::new(), ChatTracker::new());
            tracker.set_mode(mode);
            let mut out = Vec::new();
            for (f, fx) in (0u32..).zip(frames) {
                detector.detect(&left_view(&fx.frame), &mut dets).unwrap();
                tracker.push(f, &detector);
                tracker.drain_ready(&mut out);
            }
            tracker.finish(&mut out);
            for (f, ds) in out {
                per_frame[f as usize].extend(ds.iter().map(|d| d.rect));
            }
        }
        Region::VoiceChat => {
            let (mut detector, mut tracker) = (VoiceDetector::new(), VoiceTracker::new());
            tracker.set_mode(mode);
            let mut out: Vec<VoiceFrameBoxes> = Vec::new();
            for (f, fx) in (0u32..).zip(frames) {
                detector.detect(&left_view(&fx.frame), &mut dets).unwrap();
                let frame = VoiceFrame {
                    frame: f,
                    slots: detector.slots(),
                    cut: false,
                };
                tracker.push(&frame).unwrap();
                tracker.drain_ready(&mut out);
            }
            tracker.finish(&mut out);
            for b in out {
                per_frame[b.frame as usize].extend(b.boxes.iter().map(|b| b.rect));
            }
        }
        _ => {
            let (mut detector, mut tracker) = (BannerDetector::new(), BannerTracker::new());
            tracker.set_mode(mode);
            let mut out: Vec<BannerBox> = Vec::new();
            for (f, fx) in (0u32..).zip(frames) {
                let seen = detector.detect(&feed_view(&fx.frame)).unwrap();
                tracker.push(f, &seen, &mut out);
            }
            tracker.finish(&mut out);
            for b in out {
                per_frame[b.frame as usize].push(b.rect);
            }
        }
    }
    per_frame
}

/// The frames in which a name of `region` is not fully covered by
/// `boxes`.
fn leaks(frames: &[Fixture], region: Region, boxes: &[Vec<RectI>]) -> Vec<u32> {
    (0u32..)
        .zip(frames)
        .zip(boxes)
        .filter(|((_, fx), rects)| {
            let prepared = prepared_rects(&fx.frame, rects);
            item_coverage(&fx.truth, &prepared)
                .iter()
                .any(|c| c.region == region && c.is_leak())
        })
        .map(|((f, _), _)| f)
        .collect()
}

/// The over-blur of `boxes` over all frames.
fn over_blur_of(frames: &[Fixture], boxes: &[Vec<RectI>]) -> OverBlur {
    let mut total = OverBlur::default();
    for (fx, rects) in frames.iter().zip(boxes) {
        let o = over_blur(&fx.truth, &prepared_rects(&fx.frame, rects));
        total.blurred += o.blurred;
        total.outside += o.outside;
    }
    total
}

#[test]
fn safe_mode_covers_every_name_with_bounded_over_blur() {
    let frames = render_all(&scene(), |fx| fx);
    let mut all = [
        vec![Vec::new(); frames.len()],
        vec![Vec::new(); frames.len()],
    ];
    for region in [
        Region::KillFeed,
        Region::TextChat,
        Region::VoiceChat,
        Region::Banner,
    ] {
        let names = frames
            .iter()
            .filter(|fx| fx.truth.in_region(region).next().is_some())
            .count();
        assert!(names >= 40, "{region:?}: {names} name frames");
        for (mode, all) in [Mode::Tight, Mode::Safe].into_iter().zip(&mut all) {
            let boxes = boxes(&frames, region, mode);
            assert_eq!(
                leaks(&frames, region, &boxes),
                [0u32; 0],
                "{region:?} {mode:?}"
            );
            for (a, b) in all.iter_mut().zip(boxes) {
                a.extend(b);
            }
        }
    }
    let [tight, safe] = all.map(|boxes| over_blur_of(&frames, &boxes));
    assert!(
        safe.blurred > tight.blurred,
        "Safe blurs no more than Tight"
    );
    assert!(
        safe.ratio() <= MAX_SAFE_OVER_BLUR,
        "Safe over-blur {:.3} (Tight {:.3})",
        safe.ratio(),
        tight.ratio()
    );
}
