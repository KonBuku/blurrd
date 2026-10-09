//! The voice-chat detector and tracker on synthetic sequences: every frame
//! that shows a speaker name is fully covered, through flashes that wash
//! out the entries and through blink-offs, and no box outlives an entry by
//! more than the post-roll or crosses a scene cut.

#![allow(
    clippy::unwrap_used,
    reason = "test helpers on small frames: a panic is a failure"
)]

mod support;

use blurrd_blur::{PixelParams, PreparedBox, YuvColour, prepare_boxes};
use blurrd_core::{BlurBox, BlurParams, Confidence, FrameRange, Region};
use blurrd_detect::scene_cut::SceneCutDetector;
use blurrd_detect::voice::{LEFT_PANEL_ROI, POST_ROLL, VoiceFrame, VoiceFrameBoxes};
use blurrd_detect::{RgbView, VoiceDetector, VoiceTracker};
use blurrd_testkit::hud::voice::{self, Speaker, VoiceSpec};
use blurrd_testkit::metrics::item_coverage;
use blurrd_testkit::{
    Background, Fixture, Flash, FontSet, Layout, NameStyle, Palette, RgbFrame, Rng, Scene,
    synth_name,
};

/// The [`LEFT_PANEL_ROI`] of a reference-size frame, without copying.
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

/// The whole frame, the screen view of the scene-cut detector.
fn screen_view(frame: &RgbFrame) -> RgbView<'_> {
    RgbView::new(
        frame.data(),
        frame.width(),
        frame.height(),
        3 * frame.width() as usize,
    )
    .unwrap()
}

/// The blur boxes of one frame of tracker output, as the render prepares
/// them.
fn prepared(frame: &RgbFrame, boxes: &VoiceFrameBoxes) -> Vec<PreparedBox> {
    let params = PixelParams::new(&BlurParams::default(), 1.0, YuvColour::default()).unwrap();
    let boxes: Vec<BlurBox> = boxes
        .boxes
        .iter()
        .map(|b| BlurBox {
            rect: b.rect,
            region: Region::VoiceChat,
            seed: 1,
            grid_origin: (b.text.x0, b.text.y0),
            block_scale: b.confidence.block_scale(),
        })
        .collect();
    let mut out = vec![];
    prepare_boxes(&boxes, frame.width(), frame.height(), &params, &mut out);
    out
}

/// Runs the detector, the scene-cut detector and the tracker over
/// `frames` and returns the tracker's boxes per frame.
fn track(frames: &[Fixture]) -> Vec<VoiceFrameBoxes> {
    let mut detector = VoiceDetector::new();
    let mut cuts = SceneCutDetector::default();
    let mut tracker = VoiceTracker::new();
    let mut detections = vec![];
    let mut out = vec![];
    for (i, fx) in frames.iter().enumerate() {
        let left = left_view(&fx.frame);
        detector.detect(&left, &mut detections).unwrap();
        let cut = cuts.push(&screen_view(&fx.frame), &[(Region::VoiceChat, left)]);
        tracker
            .push(&VoiceFrame {
                frame: i as u32,
                slots: detector.slots(),
                cut: i > 0 && cut.is_cut_for(Region::VoiceChat),
            })
            .unwrap();
        tracker.drain_ready(&mut out);
    }
    tracker.finish(&mut out);
    assert_eq!(out.len(), frames.len());
    out
}

/// Panics if a speaker name of any frame is not fully covered.
fn assert_covered(frames: &[Fixture], boxes: &[VoiceFrameBoxes], context: &str) {
    for (fx, b) in frames.iter().zip(boxes) {
        let leaks: Vec<_> = item_coverage(&fx.truth, &prepared(&fx.frame, b))
            .into_iter()
            .filter(|c| c.region == Region::VoiceChat && c.is_leak())
            .collect();
        assert!(
            leaks.is_empty(),
            "{context}, frame {}: {leaks:?}, boxes {:?}",
            b.frame,
            b.boxes
        );
    }
}

/// Speakers in slots 0 to 2: slot 1 blinks off for 8 frames (frames 15
/// to 22), slot 2 leaves for 16 frames (20 to 35).
fn speakers(seed: u64) -> Vec<Speaker> {
    let mut rng = Rng::new(seed);
    let shown = [
        vec![FrameRange { f0: 2, f1: 33 }],
        vec![FrameRange { f0: 2, f1: 14 }, FrameRange { f0: 23, f1: 33 }],
        vec![FrameRange { f0: 4, f1: 19 }, FrameRange { f0: 36, f1: 42 }],
    ];
    (0..3u8)
        .zip(shown)
        .map(|(slot, shown)| Speaker {
            name: synth_name(&mut rng, NameStyle::LATIN_MIX[usize::from(slot) * 2]),
            location: voice::LOCATIONS[usize::from(slot)].to_owned(),
            slot,
            shown,
        })
        .collect()
}

#[test]
fn every_name_frame_is_covered_through_flashes_and_blinks() {
    let fonts = FontSet::latin();
    for (seed, background) in [Background::noise(1), Background::dusk()]
        .into_iter()
        .enumerate()
    {
        let scene = Scene::new(Layout::reference(), background, Palette::DEFAULT, 48)
            .with_voice(
                &fonts,
                VoiceSpec {
                    speakers: speakers(seed as u64),
                },
            )
            .with_flash(Flash {
                start: 10,
                frames: 5,
                amount: 0.85,
                area: None,
            });
        let frames: Vec<Fixture> = scene.iter().collect();
        let boxes = track(&frames);
        let context = format!("{background:?}");
        assert_covered(&frames, &boxes, &context);
        let in_slot = |f: usize, slot: u8| boxes[f].boxes.iter().find(|b| b.slot == slot);
        // The flash (frames 10 to 14) hides the entries from the detector;
        // the tracks bridge it.
        for b in &boxes[10..15] {
            assert_eq!(b.boxes.len(), 3, "{context}: frame {}", b.frame);
        }
        // The short blink-off in slot 1 is bridged: no flicker.
        for f in 15..23 {
            assert_eq!(
                in_slot(f, 1).map(|b| b.confidence),
                Some(Confidence::GapFill),
                "{context}: frame {f}"
            );
        }
        // The entry in slot 2 leaves for longer than a gap: one frame of
        // post-roll, then no box until it is back.
        assert_eq!(
            in_slot(20, 2).map(|b| b.confidence),
            Some(Confidence::PostRoll),
            "{context}"
        );
        for f in 20 + POST_ROLL as usize..36 {
            assert!(in_slot(f, 2).is_none(), "{context}: frame {f}");
        }
        // Nothing before the first entry, nothing after the post-roll of
        // the last.
        assert!(boxes[..2].iter().all(|b| b.boxes.is_empty()), "{context}");
        assert!(boxes[44..].iter().all(|b| b.boxes.is_empty()), "{context}");
    }
}

#[test]
fn boxes_do_not_cross_a_scene_cut() {
    // A shot with an entry that ends with the shot, then a new shot
    // without voice chat: the post-roll must not carry the box into it.
    let fonts = FontSet::latin();
    let mut rng = Rng::new(9);
    let speaker = Speaker {
        name: synth_name(&mut rng, NameStyle::Latin(9)),
        location: voice::LOCATIONS[0].to_owned(),
        slot: 0,
        shown: vec![FrameRange { f0: 0, f1: 29 }],
    };
    let first = Scene::new(
        Layout::reference(),
        Background::dusk(),
        Palette::DEFAULT,
        20,
    )
    .with_voice(
        &fonts,
        VoiceSpec {
            speakers: vec![speaker],
        },
    );
    let second = Scene::new(
        Layout::reference(),
        Background::white_sky(),
        Palette::DEFAULT,
        10,
    );
    let frames: Vec<Fixture> = first.iter().chain(second.iter()).collect();
    let boxes = track(&frames);
    assert_covered(&frames, &boxes, "cut");
    assert!(boxes[..20].iter().all(|b| b.boxes.len() == 1));
    assert!(boxes[20..].iter().all(|b| b.boxes.is_empty()));
}
