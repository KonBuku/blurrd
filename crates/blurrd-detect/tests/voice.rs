//! The voice-chat detector on synthetic frames: every speaker name in up
//! to four stacked entries is found and fully covered, two-word names
//! keep their last word, the location line below a name is never inside a
//! box, and neither colour filters nor busy backgrounds change that.

#![allow(
    clippy::unwrap_used,
    reason = "test helpers on small frames: a panic is a failure"
)]

mod support;

use blurrd_core::{FrameRange, RectI, Region};
use blurrd_detect::voice::{LEFT_PANEL_ROI, SLOTS, slot_top};
use blurrd_detect::{DetectError, Detection, RgbView, VoiceDetector};
use blurrd_testkit::font::fetched_fonts_required;
use blurrd_testkit::hud::voice::{self, Speaker, VoiceSpec};
use blurrd_testkit::metrics::{item_coverage, portrait_intrusion};
use blurrd_testkit::truth::ContentKind;
use blurrd_testkit::{
    Background, Fixture, FontSet, Layout, NameStyle, Palette, RgbFrame, Rng, Scene, Script,
    synth_name,
};

use support::{Filter, prepared};

/// Colour changes of GPU drivers and edited videos: hue shifts, black and
/// white, washed-out and vivid colours, gamma and darkened levels.
const FILTERS: [Filter; 10] = [
    Filter::Hue(120.0),
    Filter::Hue(-90.0),
    Filter::Saturation(0.0),
    Filter::Saturation(1.8),
    Filter::Vibrance(1.0),
    Filter::Gamma(0.8),
    Filter::Gamma(1.5),
    Filter::Levels {
        brightness: 0.06,
        contrast: 0.85,
    },
    Filter::Levels {
        brightness: -0.10,
        contrast: 1.0,
    },
    Filter::Levels {
        brightness: -0.12,
        contrast: 0.85,
    },
];

/// The glyph halo every box must cover (`GLYPH_HALO_REF_PX` at reference
/// scale).
const HALO: i32 = 2;

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

/// Speakers in the given slots, shown in every frame, with names of the
/// given styles.
fn speakers(rng: &mut Rng, slots: &[u8], styles: &[NameStyle]) -> Vec<Speaker> {
    slots
        .iter()
        .zip(styles.iter().cycle())
        .map(|(&slot, &style)| Speaker {
            name: synth_name(rng, style),
            location: voice::LOCATIONS[usize::from(slot)].to_owned(),
            slot,
            shown: vec![FrameRange { f0: 0, f1: 9 }],
        })
        .collect()
}

/// Frame 0 of a scene with the given speakers.
fn fixture(fonts: &FontSet, background: Background, speakers: Vec<Speaker>) -> Fixture {
    Scene::new(Layout::reference(), background, Palette::DEFAULT, 10)
        .with_voice(fonts, VoiceSpec { speakers })
        .render(0)
}

/// The voice-chat detections of one frame, checked against its truth.
#[derive(Debug, Default)]
struct Check {
    /// Speaker names in the truth.
    items: usize,
    /// What the detector reported.
    detections: Vec<Detection>,
    /// Names with glyph pixels the boxes do not fully cover.
    leaks: Vec<String>,
    /// Names no detection overlaps.
    missed: Vec<String>,
    /// Detections that overlap no speaker name.
    false_positives: Vec<String>,
    /// Detections whose box reaches into a location line.
    on_location: Vec<String>,
    /// Blurred pixels inside the portraits.
    portrait_pixels: u64,
}

impl Check {
    /// Panics with the details if a name is missed or readable, a
    /// detection is false, or a box touches a location line or portrait.
    fn assert_clean(&self, context: &str) {
        assert!(
            self.missed.is_empty()
                && self.false_positives.is_empty()
                && self.leaks.is_empty()
                && self.on_location.is_empty()
                && self.portrait_pixels == 0,
            "{context}: {self:#?}"
        );
    }
}

/// Runs `detector` on a fixture and checks every speaker name.
fn check_with(detector: &mut VoiceDetector, fx: &Fixture) -> Check {
    let mut detections = vec![];
    detector
        .detect(&left_view(&fx.frame), &mut detections)
        .unwrap();
    let boxes = prepared(&fx.frame, &detections);
    let items: Vec<_> = fx.truth.in_region(Region::VoiceChat).collect();
    let leaks = item_coverage(&fx.truth, &boxes)
        .iter()
        .filter(|c| c.region == Region::VoiceChat && c.is_leak())
        .map(|c| {
            let item = items.iter().find(|i| i.id == c.id).unwrap();
            format!(
                "{} {} ({} of {} px)",
                item.text, item.slot, c.leaked_pixels, c.glyph_pixels
            )
        })
        .collect();
    let missed = items
        .iter()
        .filter(|i| {
            !detections
                .iter()
                .any(|d| i.text.intersection(&d.text).is_some())
        })
        .map(|i| format!("{} {}", i.text, i.slot))
        .collect();
    let false_positives = detections
        .iter()
        .filter(|d| !items.iter().any(|i| i.text.intersection(&d.text).is_some()))
        .map(|d| format!("{}", d.rect))
        .collect();
    // A box may reach into the top rows of a location line only as far as
    // the name's own glyphs (deep descenders of some scripts) do, plus
    // their halo and two rows of margin for their faint tips.
    let name_bottom = |d: &Detection| {
        items
            .iter()
            .filter(|i| i.text.intersection(&d.text).is_some())
            .map(|i| i.text.y1)
            .max()
            .unwrap_or(d.text.y1)
    };
    let on_location = detections
        .iter()
        .filter(|d| {
            fx.truth.content.iter().any(|c| {
                c.kind == ContentKind::Location
                    && c.rect.intersection(&d.rect).is_some()
                    && d.rect.y1 > (name_bottom(d) + HALO + 2).max(c.rect.y0)
            })
        })
        .map(|d| format!("{}", d.rect))
        .collect();
    Check {
        items: items.len(),
        portrait_pixels: portrait_intrusion(&fx.truth, &boxes),
        detections,
        leaks,
        missed,
        false_positives,
        on_location,
    }
}

/// [`check_with`] with a fresh detector.
fn check(fx: &Fixture) -> Check {
    check_with(&mut VoiceDetector::new(), fx)
}

#[test]
fn four_stacked_entries_are_found_and_covered() {
    let fonts = FontSet::latin();
    let mut names = 0;
    for seed in 0..12u64 {
        let mut rng = Rng::new(400 + seed);
        let background = Background::variety(seed)[(seed % 5) as usize];
        let fx = fixture(
            &fonts,
            background,
            speakers(&mut rng, &[0, 1, 2, 3], &NameStyle::LATIN_MIX),
        );
        let result = check(&fx);
        result.assert_clean(&format!("seed {seed}, {background:?}"));
        assert_eq!(result.items, 4, "seed {seed}");
        names += result.items;
    }
    assert_eq!(names, 48);
}

#[test]
fn detections_are_reported_bottom_slot_first_with_their_slot_top() {
    let fonts = FontSet::latin();
    let mut rng = Rng::new(7);
    let fx = fixture(
        &fonts,
        Background::dusk(),
        speakers(&mut rng, &[2, 0, 3], &[NameStyle::Latin(9)]),
    );
    let result = check(&fx);
    result.assert_clean("three of four slots");
    let tops: Vec<_> = result.detections.iter().map(|d| d.row_top).collect();
    assert_eq!(
        tops,
        vec![Some(slot_top(0)), Some(slot_top(2)), Some(slot_top(3))]
    );
    assert!(
        result
            .detections
            .iter()
            .all(|d| d.region == Region::VoiceChat)
    );
}

#[test]
fn two_word_names_keep_their_last_word() {
    let fonts = FontSet::latin();
    // Generated two-word names, and hand-made ones with a short first
    // word, a double space and the longest name the game allows.
    let mut rng = Rng::new(31);
    let mut names: Vec<String> = (0..6)
        .map(|_| synth_name(&mut rng, NameStyle::TwoWord))
        .collect();
    names.extend(
        [
            "Jo Brakstel",
            "Tor  Venmark",
            "Ka Lu Mi",
            "Wwwwwwww Wwwwwww",
        ]
        .map(str::to_owned),
    );
    for (i, name) in names.iter().enumerate() {
        let slot = (i % usize::from(voice::SLOTS)) as u8;
        let speakers = vec![Speaker {
            name: name.clone(),
            location: voice::LOCATIONS[usize::from(slot)].to_owned(),
            slot,
            shown: vec![FrameRange { f0: 0, f1: 9 }],
        }];
        let fx = fixture(&fonts, Background::noise(i as u64), speakers);
        let result = check(&fx);
        result.assert_clean(&format!("{name:?}"));
        let item = fx.truth.in_region(Region::VoiceChat).next().unwrap();
        let found = result.detections[0];
        assert!(
            found.text.x1 >= item.text.x1 - 1,
            "{name:?}: name ends at {}, detection at {}",
            item.text.x1,
            found.text.x1
        );
    }
}

#[test]
fn the_location_line_never_enters_a_box() {
    // Long location lines start at the name's left edge and run far past
    // short names; the box must stay above them in every slot.
    let fonts = FontSet::latin();
    let mut rng = Rng::new(52);
    for seed in 0..8u64 {
        let mut entries = speakers(
            &mut rng,
            &[0, 1, 2, 3],
            &[NameStyle::Latin(3), NameStyle::Latin(16)],
        );
        for e in &mut entries {
            "in Very Long Corridor Name - Team".clone_into(&mut e.location);
        }
        let fx = fixture(
            &fonts,
            Background::variety(seed)[(seed % 5) as usize],
            entries,
        );
        let result = check(&fx);
        result.assert_clean(&format!("seed {seed}"));
        let locations: Vec<RectI> = fx
            .truth
            .content
            .iter()
            .filter(|c| c.kind == ContentKind::Location)
            .map(|c| c.rect)
            .collect();
        assert_eq!(locations.len(), 4);
        for d in &result.detections {
            for l in &locations {
                assert!(d.rect.y1 < l.y0 || d.rect.y0 >= l.y1, "{} vs {l}", d.rect);
            }
        }
    }
}

#[test]
fn colour_filters_leave_no_name_readable() {
    let fonts = FontSet::latin();
    let mut names = 0;
    for (f, filter) in FILTERS.iter().enumerate() {
        for seed in 0..4u64 {
            let mut rng = Rng::new(600 + 10 * f as u64 + seed);
            let background = Background::variety(seed + f as u64)[((seed + f as u64) % 5) as usize];
            let fx = fixture(
                &fonts,
                background,
                speakers(&mut rng, &[0, 1, 2, 3], &NameStyle::LATIN_MIX),
            );
            let fx = Fixture {
                frame: filter.whole_frame(&fx.frame),
                truth: fx.truth,
            };
            let result = check(&fx);
            result.assert_clean(&format!("{filter:?}, seed {seed}, {background:?}"));
            names += result.items;
        }
    }
    assert_eq!(names, 160);
}

#[test]
fn colour_blind_palettes_do_not_change_the_result() {
    // The voice entries carry no team colour, so the palettes must not
    // matter; this guards against a detector that starts to use one.
    let fonts = FontSet::latin();
    for palette in Palette::ALL {
        let mut rng = Rng::new(70);
        let speakers = speakers(&mut rng, &[0, 1, 2, 3], &NameStyle::LATIN_MIX);
        let fx = Scene::new(Layout::reference(), Background::noise(9), palette, 4)
            .with_voice(&fonts, VoiceSpec { speakers })
            .render(1);
        check(&fx).assert_clean(palette.name);
    }
}

#[test]
fn busy_backgrounds_without_entries_report_nothing() {
    let fonts = FontSet::latin();
    let mut detector = VoiceDetector::new();
    for seed in 0..10u64 {
        for background in Background::variety(seed) {
            let fx = fixture(&fonts, background, vec![]);
            let result = check_with(&mut detector, &fx);
            assert!(
                result.detections.is_empty(),
                "{background:?}: {:?}",
                result.detections
            );
            assert!(detector.slots().iter().all(|s| !s.is_entry()));
        }
    }
}

#[test]
fn blink_offs_leave_the_slot_empty() {
    let fonts = FontSet::latin();
    let mut rng = Rng::new(81);
    let speakers = vec![Speaker {
        name: synth_name(&mut rng, NameStyle::Latin(8)),
        location: voice::LOCATIONS[1].to_owned(),
        slot: 1,
        shown: vec![FrameRange { f0: 0, f1: 3 }, FrameRange { f0: 9, f1: 12 }],
    }];
    let scene = Scene::new(
        Layout::reference(),
        Background::noise(2),
        Palette::DEFAULT,
        13,
    )
    .with_voice(&fonts, VoiceSpec { speakers });
    let mut detector = VoiceDetector::new();
    for (frame, fx) in scene.iter().enumerate() {
        let result = check_with(&mut detector, &fx);
        result.assert_clean(&format!("frame {frame}"));
        let shown = !(4..9).contains(&frame);
        assert_eq!(result.detections.len(), usize::from(shown), "frame {frame}");
        assert_eq!(detector.slots()[1].is_entry(), shown, "frame {frame}");
    }
}

#[test]
fn names_in_other_scripts_are_covered() {
    let fonts = match FontSet::with_fetched() {
        Ok(fonts) => fonts,
        Err(missing) if !fetched_fonts_required() => {
            eprintln!("skipping: {missing}");
            return;
        }
        Err(missing) => panic!("{missing}"),
    };
    let mut rng = Rng::new(91);
    for (i, script) in Script::FETCHED.into_iter().enumerate() {
        let slot = (i % usize::from(voice::SLOTS)) as u8;
        let speakers = vec![Speaker {
            name: synth_name(&mut rng, NameStyle::Script(script)),
            location: voice::LOCATIONS[usize::from(slot)].to_owned(),
            slot,
            shown: vec![FrameRange { f0: 0, f1: 9 }],
        }];
        let fx = fixture(&fonts, Background::noise(i as u64), speakers);
        check(&fx).assert_clean(&format!("{script:?}"));
    }
}

#[test]
fn every_slot_is_measured_in_every_frame() {
    let fonts = FontSet::latin();
    let mut rng = Rng::new(3);
    let fx = fixture(
        &fonts,
        Background::noise(1),
        speakers(&mut rng, &[1], &[NameStyle::Latin(6)]),
    );
    let mut detector = VoiceDetector::new();
    check_with(&mut detector, &fx).assert_clean("slot 1");
    let slots = detector.slots();
    assert_eq!(slots.len(), SLOTS);
    for (k, s) in slots.iter().enumerate() {
        assert_eq!(usize::from(s.slot), k);
        assert_eq!(s.top, slot_top(k));
        assert_eq!(s.is_entry(), k == 1, "slot {k}: {s:?}");
    }
    // The portrait ink is the structure signal of an entry.
    assert!(slots[1].portrait >= 1.0 && slots[0].portrait < 1.0);
}

#[test]
fn rejects_crops_of_the_wrong_size() {
    let data = vec![0u8; 3 * 100 * 100];
    let view = RgbView::new(&data, 100, 100, 300).unwrap();
    let mut out = vec![];
    assert!(matches!(
        VoiceDetector::new().detect(&view, &mut out),
        Err(DetectError::CropSize { width: 780, .. })
    ));
}
