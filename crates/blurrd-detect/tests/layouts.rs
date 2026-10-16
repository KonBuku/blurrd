//! HUD geometry calibration and normalised reading on synthetic frames of
//! every layout of the fixture matrix (16:9 at 720p, 1080p, 1440p and 4K,
//! native 4:3, 4:3 pillarboxed, 16:10 and 4:3 stretched) and every
//! colour-blind palette: the calibration finds the layout and learns the
//! kill-feed bar colours of the palettes, and every kill-feed, chat, voice
//! and banner name read through it is fully covered in source pixels
//! (plan 7.3, M15).

#![allow(
    clippy::cast_possible_wrap,
    clippy::unwrap_used,
    reason = "test helpers on small frames: coordinates never wrap, a panic is a failure"
)]

mod support;

use blurrd_blur::{PixelParams, PreparedBox, YuvColour, prepare_boxes};
use blurrd_core::{BlurBox, BlurParams, FrameRange, HudAnchor, HudLayout, NameKind, Region};
use blurrd_detect::calibration::{Calibrator, MIN_CONFIDENCE};
use blurrd_detect::chat::LEFT_PANEL_ROI;
use blurrd_detect::geometry::{HudCrop, HudSampler};
use blurrd_detect::killfeed::FEED_ROI;
use blurrd_detect::temporal::killfeed::{KillFeedFrame, KillFeedTracker};
use blurrd_detect::voice::LEFT_PANEL_ROI as VOICE_PANEL_ROI;
use blurrd_detect::{
    BannerDetector, BannerTracker, ChatDetector, Detection, KillFeedDetector, RgbView,
    VoiceDetector, banner,
};
use blurrd_testkit::hud::banner::BannerSpec;
use blurrd_testkit::hud::chat::{Channel, ChatInput, ChatMessage, ChatSpec, Language};
use blurrd_testkit::hud::killfeed::{Kill, KillFeedSpec, Team, Weapon};
use blurrd_testkit::hud::voice::{self, Speaker, VoiceSpec};
use blurrd_testkit::metrics::item_coverage;
use blurrd_testkit::{
    Background, Fixture, FontSet, Layout, NameStyle, Palette, RgbFrame, Rng, Scene, synth_name,
};

use support::Filter;

/// Frames of every scene: kills and messages arrive in the first 12.
const FRAMES: u32 = 40;
/// Every this many frames is read by the calibrator.
const SAMPLE_STEP: u32 = 4;

/// A scene with four kills and three chat messages in `layout`.
fn scene(layout: Layout, palette: Palette, seed: u64) -> Scene {
    let fonts = FontSet::latin();
    let mut rng = Rng::new(seed);
    let kills = (0..4)
        .map(|i| Kill {
            killer: synth_name(&mut rng, NameStyle::Latin(5 + i as usize * 3)),
            victim: synth_name(&mut rng, NameStyle::Latin(14 - i as usize * 3)),
            killer_team: if i % 2 == 0 { Team::Ally } else { Team::Enemy },
            victim_team: if i % 2 == 0 { Team::Enemy } else { Team::Ally },
            weapon: [Weapon::Rifle, Weapon::Pistol, Weapon::Knife, Weapon::Rifle][i as usize],
            badge: None,
            own: false,
            headshot: i % 2 == 1,
            birth: 2 * i,
        })
        .collect();
    let messages = [Channel::All, Channel::Team, Channel::Broadcast]
        .into_iter()
        .enumerate()
        .map(|(i, channel)| ChatMessage {
            channel,
            name: synth_name(&mut rng, NameStyle::Latin(6 + 3 * i)),
            team: Team::Ally,
            text: "rotate mid now".to_owned(),
            role: (channel == Channel::Broadcast).then(|| "Sova".to_owned()),
            wrap: false,
            time: 1 + 3 * i as u32,
        })
        .collect();
    Scene::new(layout, Background::dusk(), palette, FRAMES)
        .with_kill_feed(&fonts, KillFeedSpec::new(kills))
        .with_chat(&fonts, ChatSpec::new(Language::English, messages))
}

/// The whole frame as a view.
fn view(frame: &RgbFrame) -> RgbView<'_> {
    RgbView::new(
        frame.data(),
        frame.width(),
        frame.height(),
        3 * frame.width() as usize,
    )
    .unwrap()
}

/// The kill-feed and chat detections of `fx`, read through `sampler` and
/// mapped to source pixels.
fn detect(sampler: &HudSampler, fx: &Fixture) -> Vec<Detection> {
    let frame = view(&fx.frame);
    let mut crop = HudCrop::default();
    let mut found = Vec::new();
    let mut dets = Vec::new();
    let feed = sampler.crop(&frame, FEED_ROI, HudAnchor::Right, &mut crop);
    KillFeedDetector::new().detect(&feed, &mut dets).unwrap();
    found.extend(dets.iter().map(|d| to_source(sampler, d, HudAnchor::Right)));
    let left = sampler.crop(&frame, LEFT_PANEL_ROI, HudAnchor::Left, &mut crop);
    ChatDetector::new().detect(&left, &mut dets).unwrap();
    found.extend(dets.iter().map(|d| to_source(sampler, d, HudAnchor::Left)));
    found
}

/// A detection in source pixels.
fn to_source(sampler: &HudSampler, d: &Detection, anchor: HudAnchor) -> Detection {
    Detection {
        text: sampler.to_source(d.text, anchor),
        rect: sampler.to_source(d.rect, anchor),
        ..*d
    }
}

/// The blur boxes of `detections` in a frame of `fx`'s size, with the
/// default blur at the layout's scale.
fn prepared(fx: &Fixture, scale: f32, detections: &[Detection]) -> Vec<PreparedBox> {
    let params = PixelParams::new(&BlurParams::default(), scale, YuvColour::default()).unwrap();
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
    prepare_boxes(
        &boxes,
        fx.frame.width(),
        fx.frame.height(),
        &params,
        &mut out,
    );
    out
}

/// The fixture matrix, and 16:9 games saved at 4:3 frame sizes.
fn layouts() -> Vec<Layout> {
    let scaled = [
        Layout::scaled_16x9(1440, 1080),
        Layout::scaled_16x9(1024, 768),
    ];
    Layout::matrix().into_iter().chain(scaled).collect()
}

#[test]
fn calibration_finds_every_layout_and_its_names_are_covered() {
    for (k, layout) in (0u64..).zip(layouts()) {
        for (p, palette) in (0u64..).zip(Palette::ALL) {
            let scene = scene(layout, palette, 10 * k + p);
            let mut calibrator = Calibrator::new(layout.width, layout.height, HudLayout::Auto);
            for f in (0..FRAMES).step_by(SAMPLE_STEP as usize) {
                calibrator.push(f, &view(&scene.render(f).frame));
            }
            let calibration = calibrator.finish();
            assert_eq!(calibration.segments.len(), 1, "{layout:?}");
            let geometry = calibration.geometry_at(0).unwrap().clone();
            let what = format!(
                "{:?} {}x{} palette {p}",
                layout.kind, layout.width, layout.height
            );
            assert_eq!(geometry.layout, layout.kind, "{what}");
            assert!(
                geometry.confidence >= MIN_CONFIDENCE,
                "{what}: {geometry:?}"
            );
            assert!((geometry.sx - layout.sx).abs() < 1e-3, "{what}");
            assert!((geometry.sy - layout.sy).abs() < 1e-3, "{what}");
            // Colour-blind palettes learn their bar colours; the default
            // palette keeps the default classes.
            let hues = calibration.segments[0].hues;
            assert_eq!(
                hues.is_some(),
                palette != Palette::DEFAULT,
                "{what}: {hues:?}"
            );
            let sampler = HudSampler::new(geometry);
            // Every name in the settled frames is covered.
            for f in [16, 24, 32] {
                let fx = scene.render(f);
                let detections = detect(&sampler, &fx);
                let boxes = prepared(&fx, layout.sy, &detections);
                let leaks: Vec<String> = item_coverage(&fx.truth, &boxes)
                    .iter()
                    .filter(|c| {
                        matches!(c.region, Region::KillFeed | Region::TextChat) && c.is_leak()
                    })
                    .map(|c| {
                        let item = fx.truth.items.iter().find(|i| i.id == c.id).unwrap();
                        format!(
                            "{} ({} of {} px)",
                            item.text, c.leaked_pixels, c.glyph_pixels
                        )
                    })
                    .collect();
                let names = fx
                    .truth
                    .items
                    .iter()
                    .filter(|i| matches!(i.region, Region::KillFeed | Region::TextChat))
                    .count();
                assert!(names >= 8, "{what} frame {f}: {names} names");
                assert!(leaks.is_empty(), "{what} frame {f}: {leaks:?}");
            }
        }
    }
}

#[test]
fn a_frame_without_hud_is_calibrated_with_low_confidence() {
    let layout = Layout::reference();
    let scene = Scene::new(layout, Background::noise(5), Palette::DEFAULT, 8);
    let mut calibrator = Calibrator::new(layout.width, layout.height, HudLayout::Auto);
    for f in 0..8 {
        calibrator.push(f, &view(&scene.render(f).frame));
    }
    let calibration = calibrator.finish();
    assert!(calibration.is_low());
    let geometry = calibration.geometry_at(0).unwrap();
    assert_eq!(geometry.layout, HudLayout::Wide16x9);
    assert!(geometry.confidence < MIN_CONFIDENCE);
}

#[test]
fn a_change_of_bars_starts_a_new_segment() {
    let wide = scene(Layout::reference(), Palette::DEFAULT, 1);
    let boxed = scene(Layout::pillarbox(1920, 1080), Palette::DEFAULT, 2);
    let mut calibrator = Calibrator::new(1920, 1080, HudLayout::Auto);
    for f in 0..FRAMES {
        let s = if f < FRAMES / 2 { &wide } else { &boxed };
        calibrator.push(f, &view(&s.render(f % (FRAMES / 2) + 12).frame));
    }
    let calibration = calibrator.finish();
    assert_eq!(calibration.segments.len(), 2, "{calibration:?}");
    assert_eq!(calibration.segments[1].first_frame, FRAMES / 2);
    assert_eq!(
        calibration.geometry_at(FRAMES - 1).unwrap().layout,
        HudLayout::Pillarbox { bar_px: 240 }
    );
    assert_eq!(
        calibration.geometry_at(0).unwrap().layout,
        HudLayout::Wide16x9
    );
}

#[test]
fn two_word_chat_names_are_covered_in_greyscale_720p() {
    // Native 720p video in greyscale with the chat open on a bright
    // background: the enemies' red names and prefixes fade towards the
    // background, and every two-word name is still covered whole. (A
    // recording where a broken letter of such a name read as the colon
    // needs the game's font at 720p; the fixture fonts do not break that
    // way, so this guards the surrounding behaviour only.)
    let layout = Layout::wide(1280, 720);
    let sampler = HudSampler::new(layout.geometry());
    let fonts = FontSet::latin();
    for seed in 0..6 {
        let mut rng = Rng::new(1500 + seed);
        let messages = (0..8)
            .map(|i| ChatMessage {
                channel: Channel::All,
                name: synth_name(&mut rng, NameStyle::TwoWord),
                team: if i % 2 == 0 { Team::Enemy } else { Team::Ally },
                text: "nice try".to_owned(),
                role: None,
                wrap: false,
                time: 1 + 2 * i,
            })
            .collect();
        let spec = ChatSpec {
            input: Some(ChatInput {
                from: 20,
                to: 40,
                channel: Channel::All,
                text: "gg".to_owned(),
            }),
            ..ChatSpec::new(Language::English, messages)
        };
        let scene = Scene::new(layout, Background::white_sky(), Palette::DEFAULT, FRAMES)
            .with_chat(&fonts, spec);
        let fx = scene.render(30);
        let fx = Fixture {
            frame: Filter::Saturation(0.0).whole_frame(&fx.frame),
            truth: fx.truth,
        };
        let frame = view(&fx.frame);
        let mut crop = HudCrop::default();
        let left = sampler.crop(&frame, LEFT_PANEL_ROI, HudAnchor::Left, &mut crop);
        let mut dets = Vec::new();
        ChatDetector::new().detect(&left, &mut dets).unwrap();
        let detections: Vec<Detection> = dets
            .iter()
            .map(|d| to_source(&sampler, d, HudAnchor::Left))
            .collect();
        let boxes = prepared(&fx, layout.sy, &detections);
        let coverage = item_coverage(&fx.truth, &boxes);
        let names: Vec<_> = coverage
            .iter()
            .filter(|c| c.region == Region::TextChat)
            .collect();
        assert_eq!(names.len(), 8, "seed {seed}");
        let leaks: Vec<String> = names
            .iter()
            .filter(|c| c.is_leak())
            .map(|c| {
                let item = fx.truth.items.iter().find(|i| i.id == c.id).unwrap();
                format!(
                    "{} ({} of {} px)",
                    item.text, c.leaked_pixels, c.glyph_pixels
                )
            })
            .collect();
        assert!(leaks.is_empty(), "seed {seed}: {leaks:?}");
    }
}

#[test]
fn voice_names_are_covered_in_every_layout() {
    // Three speakers of every name style on dark, textured and bright
    // backgrounds, read through the left-anchored panel crop.
    let fonts = FontSet::latin();
    let styles = [
        NameStyle::Latin(3),
        NameStyle::Latin(8),
        NameStyle::Latin(14),
        NameStyle::TwoWord,
        NameStyle::Latin(16),
        NameStyle::Latin(5),
    ];
    for (k, layout) in Layout::matrix().into_iter().enumerate() {
        let sampler = HudSampler::new(layout.geometry());
        let mut detector = VoiceDetector::new();
        let mut crop = HudCrop::default();
        let mut dets = Vec::new();
        for seed in 0..12u64 {
            let mut rng = Rng::new(100 * k as u64 + seed);
            let speakers = (0..3u8)
                .map(|slot| Speaker {
                    name: synth_name(
                        &mut rng,
                        styles[(seed as usize + slot as usize) % styles.len()],
                    ),
                    location: voice::LOCATIONS[slot as usize].to_owned(),
                    slot,
                    shown: vec![FrameRange { f0: 0, f1: 9 }],
                })
                .collect();
            let background = [
                Background::dusk(),
                Background::noise(seed),
                Background::white_sky(),
            ][seed as usize % 3];
            let fx = Scene::new(layout, background, Palette::DEFAULT, 10)
                .with_voice(&fonts, VoiceSpec { speakers })
                .render(4);
            let frame = view(&fx.frame);
            let panel = sampler.crop(&frame, VOICE_PANEL_ROI, HudAnchor::Left, &mut crop);
            detector.detect(&panel, &mut dets).unwrap();
            let detections: Vec<Detection> = dets
                .iter()
                .map(|d| to_source(&sampler, d, HudAnchor::Left))
                .collect();
            let boxes = prepared(&fx, layout.sy, &detections);
            let names: Vec<_> = item_coverage(&fx.truth, &boxes)
                .into_iter()
                .filter(|c| c.region == Region::VoiceChat)
                .collect();
            let leaks: Vec<_> = names
                .iter()
                .filter(|c| c.is_leak())
                .map(|c| (c.leaked_pixels, c.glyph_pixels))
                .collect();
            assert_eq!(names.len(), 3, "{:?} seed {seed}", layout.kind);
            assert!(leaks.is_empty(), "{:?} seed {seed}: {leaks:?}", layout.kind);
        }
    }
}

#[test]
fn banner_names_are_covered_in_every_layout() {
    // An ACE banner read through the centred crop of the feed area, over
    // its whole episode (slide-in, settle, hold, fade).
    let fonts = FontSet::latin();
    for (k, layout) in layouts().into_iter().enumerate() {
        let sampler = HudSampler::new(layout.geometry());
        let name = synth_name(&mut Rng::new(3 + k as u64), NameStyle::Latin(9));
        let spec = BannerSpec {
            hold: 60,
            ..BannerSpec::new("ACE", &name, 5)
        };
        let scene = Scene::new(
            layout,
            Background::dusk(),
            Palette::DEFAULT,
            spec.end() + 20,
        )
        .with_banner(&fonts, spec);
        let fixtures: Vec<Fixture> = (0..scene.frames()).map(|f| scene.render(f)).collect();
        let mut detector = BannerDetector::new();
        let mut tracker = BannerTracker::new();
        let mut crop = HudCrop::default();
        let mut boxes = Vec::new();
        for (f, fx) in fixtures.iter().enumerate() {
            let frame = view(&fx.frame);
            let feed = sampler.crop(&frame, FEED_ROI, banner::ANCHOR, &mut crop);
            let seen = detector.detect(&feed).unwrap();
            tracker.push(f as u32, &seen, &mut boxes);
        }
        tracker.finish(&mut boxes);
        let mut names = 0;
        for (f, fx) in fixtures.iter().enumerate() {
            let detections: Vec<Detection> = boxes
                .iter()
                .filter(|b| b.frame as usize == f)
                .map(|b| {
                    let rect = sampler.to_source(b.rect, banner::ANCHOR);
                    Detection {
                        region: Region::Banner,
                        kind: NameKind::Banner,
                        text: rect,
                        rect,
                        row_top: None,
                    }
                })
                .collect();
            for c in item_coverage(&fx.truth, &prepared(fx, layout.sy, &detections)) {
                if c.region == Region::Banner {
                    names += 1;
                    assert!(!c.is_leak(), "{:?} frame {f}: {c:?}", layout.kind);
                }
            }
        }
        assert!(names > 60, "{:?}: {names} banner name frames", layout.kind);
    }
}

#[test]
fn voice_entries_are_found_at_720p_on_texture() {
    // Native 720p upscaled to the reference: on fine texture the location
    // line loses its first word ("in" and a one-letter site) to the
    // stem threshold, and the entry must still count as aligned.
    let layout = Layout::wide(1280, 720);
    let sampler = HudSampler::new(layout.geometry());
    let fonts = FontSet::latin();
    let mut detector = VoiceDetector::new();
    let mut crop = HudCrop::default();
    let mut dets = Vec::new();
    for seed in 0..60u64 {
        let mut rng = Rng::new(700 + seed);
        let speakers = (0..3u8)
            .map(|slot| Speaker {
                name: synth_name(&mut rng, NameStyle::Latin(5 + 3 * slot as usize)),
                location: voice::LOCATIONS[slot as usize].to_owned(),
                slot,
                shown: vec![FrameRange { f0: 0, f1: 9 }],
            })
            .collect();
        let fx = Scene::new(layout, Background::noise(seed), Palette::DEFAULT, 10)
            .with_voice(&fonts, VoiceSpec { speakers })
            .render(4);
        let frame = view(&fx.frame);
        let panel = sampler.crop(&frame, VOICE_PANEL_ROI, HudAnchor::Left, &mut crop);
        detector.detect(&panel, &mut dets).unwrap();
        assert_eq!(dets.len(), 3, "seed {seed}");
    }
}

/// Births of the rows of [`tracked_feed_leaks`]: two of them close
/// together, so that one row jumps up while the next slides in.
const FEED_BIRTHS: [u32; 4] = [2, 14, 18, 33];

/// The kill-feed names left readable in source pixels when a video in
/// `layout`, changed by `filter`, runs through the detector and the tracker
/// on crops of `sampler`, as `frame: name` strings, with the number of
/// name-frames checked. Every name at full opacity counts, also while its
/// row slides in. With `late`, the detector's readings of each new row are
/// dropped until it has slid into place, as on video where the row is
/// read only once it has settled: the tracker covers the slide-in from
/// the row's first frames.
fn tracked_feed_leaks(
    layout: Layout,
    filter: Filter,
    background: Background,
    seed: u64,
    late: bool,
) -> (usize, Vec<String>) {
    const VIDEO_FRAMES: u32 = 70;
    /// Frames from a row's birth until it has slid into place.
    const SLIDE: u32 = 4;
    let fonts = FontSet::latin();
    let mut rng = Rng::new(seed);
    let kills = FEED_BIRTHS
        .into_iter()
        .map(|birth| support::random_kill(&mut rng, birth))
        .collect();
    let scene = Scene::new(layout, background, Palette::DEFAULT, VIDEO_FRAMES)
        .with_kill_feed(&fonts, KillFeedSpec::new(kills));
    let sampler = HudSampler::new(layout.geometry());
    let mut detector = KillFeedDetector::new();
    let mut tracker = KillFeedTracker::new();
    let mut crop = HudCrop::default();
    let mut dets = Vec::new();
    let mut out = Vec::new();
    let mut fixtures = Vec::new();
    for f in 0..VIDEO_FRAMES {
        let fx = scene.render(f);
        let fx = Fixture {
            frame: filter.whole_frame(&fx.frame),
            truth: fx.truth,
        };
        let frame = view(&fx.frame);
        let feed = sampler.crop(&frame, FEED_ROI, HudAnchor::Right, &mut crop);
        detector.detect(&feed, &mut dets).unwrap();
        if late && FEED_BIRTHS.iter().any(|&b| (b..b + SLIDE).contains(&f)) {
            // The newest row is the lowest one.
            let newest = dets.iter().filter_map(|d| d.row_top).max();
            dets.retain(|d| d.row_top != newest);
        }
        tracker
            .push(&KillFeedFrame {
                frame: f,
                crop: feed,
                detections: &dets,
            })
            .unwrap();
        tracker.drain_ready(&mut out);
        fixtures.push(fx);
    }
    tracker.finish(&mut out);
    let mut checked = 0;
    let mut leaks = Vec::new();
    for (entry, fx) in out.iter().zip(&fixtures) {
        let detections: Vec<Detection> = entry
            .boxes
            .iter()
            .map(|b| {
                let rect = sampler.to_source(b.rect, HudAnchor::Right);
                Detection {
                    region: Region::KillFeed,
                    kind: b.kind,
                    text: rect,
                    rect,
                    row_top: None,
                }
            })
            .collect();
        let boxes = prepared(fx, layout.sy, &detections);
        for c in item_coverage(&fx.truth, &boxes) {
            let item = fx.truth.items.iter().find(|i| i.id == c.id).unwrap();
            if item.region != Region::KillFeed || item.opacity < 1.0 {
                continue;
            }
            checked += 1;
            if c.is_leak() {
                leaks.push(format!(
                    "{}: {:?} ({} of {} px)",
                    entry.frame, item.id, c.leaked_pixels, c.glyph_pixels
                ));
            }
        }
    }
    (checked, leaks)
}

#[test]
fn new_kill_feed_rows_are_covered_at_720p_under_gamma() {
    // Native 720p video brightened by a gamma filter: the bars of a new row
    // change the area they slide into less than on plain video, and the
    // names reach left of their settled place while the row slides in. A
    // row read only once it has settled is still covered from its first
    // frames (the real case also lost the template while the row slid
    // in; the fixture's rows are followed back, so this guards the
    // outcome), and right-aligned victim names whose last glyph fades are
    // covered up to the portrait.
    let layout = Layout::wide(1280, 720);
    for (seed, (gamma, background)) in [
        (0.6, Background::white_sky()),
        (0.6, Background::teal_wall(2)),
        (0.75, Background::dusk()),
        (0.75, Background::noise(3)),
    ]
    .into_iter()
    .enumerate()
    {
        for late in [false, true] {
            let (checked, leaks) = tracked_feed_leaks(
                layout,
                Filter::Gamma(gamma),
                background,
                1700 + seed as u64,
                late,
            );
            let what = format!("gamma {gamma} seed {seed} late {late}");
            assert!(checked > 300, "{what}: {checked} name-frames");
            assert!(leaks.is_empty(), "{what}: {leaks:#?}");
        }
    }
}

#[test]
fn broadcast_sender_names_are_covered_at_720p_under_gamma() {
    // Native 720p video brightened by a gamma filter: broadcast lines have
    // no colon, and no letter of their prefix that falls apart may pass
    // for one and end the sender name. Guards the native path; the break
    // itself needs the game's font at 720p.
    let layout = Layout::wide(1280, 720);
    let sampler = HudSampler::new(layout.geometry());
    let fonts = FontSet::latin();
    for seed in 0..6 {
        let mut rng = Rng::new(1800 + seed);
        let messages = (0..6)
            .map(|i| ChatMessage {
                channel: Channel::Broadcast,
                name: synth_name(
                    &mut rng,
                    NameStyle::LATIN_MIX[i % NameStyle::LATIN_MIX.len()],
                ),
                team: if i % 2 == 0 { Team::Enemy } else { Team::Ally },
                text: "spotted a gun in Defender Side Spawn".to_owned(),
                role: Some("Reyna".to_owned()),
                wrap: false,
                time: 1 + 2 * i as u32,
            })
            .collect();
        let scene = Scene::new(layout, Background::white_sky(), Palette::DEFAULT, FRAMES)
            .with_chat(&fonts, ChatSpec::new(Language::English, messages));
        let fx = scene.render(20);
        let fx = Fixture {
            frame: Filter::Gamma(0.5).whole_frame(&fx.frame),
            truth: fx.truth,
        };
        let frame = view(&fx.frame);
        let mut crop = HudCrop::default();
        let left = sampler.crop(&frame, LEFT_PANEL_ROI, HudAnchor::Left, &mut crop);
        let mut dets = Vec::new();
        ChatDetector::new().detect(&left, &mut dets).unwrap();
        let detections: Vec<Detection> = dets
            .iter()
            .map(|d| to_source(&sampler, d, HudAnchor::Left))
            .collect();
        let boxes = prepared(&fx, layout.sy, &detections);
        let coverage = item_coverage(&fx.truth, &boxes);
        let names: Vec<_> = coverage
            .iter()
            .filter(|c| c.region == Region::TextChat)
            .collect();
        // The box shows the latest lines of the long messages.
        assert!(names.len() >= 3, "seed {seed}: {} names", names.len());
        let leaks: Vec<String> = names
            .iter()
            .filter(|c| c.is_leak())
            .map(|c| {
                let item = fx.truth.items.iter().find(|i| i.id == c.id).unwrap();
                format!(
                    "{} ({} of {} px)",
                    item.text, c.leaked_pixels, c.glyph_pixels
                )
            })
            .collect();
        assert!(leaks.is_empty(), "seed {seed}: {leaks:?}");
    }
}
