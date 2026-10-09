//! The banner detector and settle model on synthetic banners from
//! blurrd-testkit: every frame that shows a name is covered, from the
//! slide-in to the fade-out, and frames without a banner name (no banner,
//! or an "X ULT READY" notification) get no box.

#![allow(
    clippy::unwrap_used,
    reason = "test helpers on small frames: a panic is a failure"
)]

mod support;

use blurrd_core::{Confidence, RectI};
use blurrd_detect::banner::FALLBACK_HALF;
use blurrd_testkit::hud::banner::{
    BannerSpec, NAME_BASELINE, PLATE, ULT_READY_BASELINE, UltReadySpec,
};
use blurrd_testkit::{
    Background, Flash, FontSet, Layout, NameStyle, Palette, Rng, Scene, synth_name,
};

use support::{Filter, run_banner};

/// Backgrounds: textured, dark and a bright sky.
const BACKGROUNDS: [Background; 3] = [
    Background::noise(3),
    Background::dusk(),
    Background::white_sky(),
];

/// A scene with one banner for `name`, starting at frame 5.
fn banner_scene(name: &str, background: Background) -> (Scene, BannerSpec) {
    let spec = BannerSpec {
        hold: 60,
        ..BannerSpec::new("ACE", name, 5)
    };
    let scene = Scene::new(
        Layout::reference(),
        background,
        Palette::DEFAULT,
        spec.end() + 20,
    )
    .with_banner(&FontSet::latin(), spec.clone());
    (scene, spec)
}

#[test]
fn every_name_frame_is_covered_from_slide_in_to_fade_out() {
    let styles = [
        NameStyle::Latin(2),
        NameStyle::Latin(5),
        NameStyle::Latin(9),
        NameStyle::Latin(16),
        NameStyle::TwoWord,
        NameStyle::Vietnamese,
    ];
    let mut frames = 0;
    for (i, style) in styles.into_iter().enumerate() {
        for (b, background) in BACKGROUNDS.into_iter().enumerate() {
            let name = synth_name(&mut Rng::new((10 * i + b) as u64), style);
            let (scene, spec) = banner_scene(&name, background);
            let run = run_banner(&scene, None);
            let context = format!("{style:?} on background {b}");
            run.assert_clean(&context);
            assert!(run.name_frames > 70, "{context}");
            // Boxes without a name only just before the slide-in or while
            // the gate is on (a white sky keeps it on, as in the
            // prototype), plus the fade-out hold.
            let first_name = spec.settle() - spec.slide_frames;
            let gated = |f: u32| (f.saturating_sub(7)..=f).any(|g| run.seen[g as usize].gate);
            assert!(
                run.empty_boxes
                    .iter()
                    .all(|&f| (f < first_name && f + 13 >= first_name) || gated(f)),
                "{context}: {:?}",
                run.empty_boxes
            );
            frames += run.name_frames;
        }
    }
    assert!(frames > 1200, "{frames} name frames checked");
}

#[test]
fn the_settled_box_is_fixed_and_centred() {
    let name = synth_name(&mut Rng::new(4), NameStyle::Latin(10));
    let (scene, spec) = banner_scene(&name, Background::dusk());
    let run = run_banner(&scene, None);
    run.assert_clean("settled");
    let settled: Vec<_> = (spec.settle() + 12..spec.settle() + 60)
        .map(|f| run.boxes[f as usize].clone())
        .collect();
    let first = settled[0][0];
    assert_eq!(first.rect.x0 + first.rect.x1, 1920, "{:?}", first.rect);
    assert_eq!(first.conf, Confidence::Tracked);
    assert!(
        settled
            .iter()
            .all(|b| b.len() == 1 && b[0].rect == first.rect),
        "the settled box does not move"
    );
    // Slide-in frames are pre-roll, the fade-out post-roll.
    let confs: Vec<_> = run.boxes.iter().flatten().map(|b| b.conf).collect();
    assert!(confs.contains(&Confidence::PreRoll) && confs.contains(&Confidence::PostRoll));
}

#[test]
fn slide_in_frames_are_covered_to_the_plate_edge() {
    // The plate leads the sliding name: every slide-in box reaches its
    // right edge (right of the settled plate while it slides). The last
    // frames of the slide, within the centred tolerance, already get the
    // settled box with its deceleration padding.
    for (b, background) in BACKGROUNDS.into_iter().enumerate() {
        let name = synth_name(&mut Rng::new(70 + b as u64), NameStyle::Latin(12));
        let (scene, spec) = banner_scene(&name, background);
        let run = run_banner(&scene, None);
        let mut sliding = 0;
        for f in spec.settle() - spec.slide_frames..spec.settle() {
            let boxes = &run.boxes[f as usize];
            let context = format!("background {b}, frame {f}");
            assert_eq!(boxes.len(), 1, "{context}");
            if boxes[0].conf != Confidence::PreRoll {
                continue;
            }
            sliding += 1;
            let rect = boxes[0].rect;
            assert!(rect.x1 >= PLATE.x1, "{context}: {rect:?}");
            if let Some(plate) = run.seen[f as usize].plate {
                assert!(rect.x1 >= plate.x1, "{context}: {rect:?} {plate:?}");
            }
        }
        assert!(
            sliding + 3 >= spec.slide_frames,
            "background {b}: {sliding}"
        );
    }
}

#[test]
fn no_banner_no_box() {
    for background in BACKGROUNDS {
        let scene = Scene::new(Layout::reference(), background, Palette::DEFAULT, 60);
        let run = run_banner(&scene, None);
        // A uniformly white sky passes the brightness rule of the gate
        // (the prototype's), but shows no name, so nothing is blurred.
        assert_eq!(run.box_frames, 0);
        assert!(run.seen.iter().all(|s| s.name.is_none()));
    }
}

#[test]
fn ult_ready_notifications_are_not_names() {
    let fonts = FontSet::latin();
    for background in BACKGROUNDS {
        // At its place above the plate, and moved onto the name's
        // baseline, where only the plate gate tells it from a name.
        for baseline in [ULT_READY_BASELINE, NAME_BASELINE] {
            let scene = Scene::new(Layout::reference(), background, Palette::DEFAULT, 40)
                .with_ult_ready(
                    &fonts,
                    UltReadySpec {
                        baseline,
                        ..UltReadySpec::new('X', 0, 40)
                    },
                );
            let run = run_banner(&scene, None);
            assert_eq!(run.box_frames, 0, "baseline {baseline}");
            assert!(run.seen.iter().all(|s| s.name.is_none()));
        }
    }
}

#[test]
fn a_notification_inside_the_banner_box_is_not_a_name() {
    // The notification shows on the name's baseline while the box fades
    // in, before plate and name arrive.
    let name = synth_name(&mut Rng::new(7), NameStyle::Latin(8));
    for background in BACKGROUNDS {
        let spec = BannerSpec {
            hold: 60,
            ..BannerSpec::new("CLUTCH", &name, 5)
        };
        let first_name = spec.settle() - spec.slide_frames;
        let scene = Scene::new(
            Layout::reference(),
            background,
            Palette::DEFAULT,
            spec.end() + 20,
        )
        .with_banner(&FontSet::latin(), spec.clone())
        .with_ult_ready(
            &FontSet::latin(),
            UltReadySpec {
                baseline: NAME_BASELINE,
                ..UltReadySpec::new('Q', 0, first_name)
            },
        );
        let run = run_banner(&scene, None);
        assert!(
            (0..first_name as usize).all(|f| run.seen[f].name.is_none()),
            "a name before the plate"
        );
        // Only the slide-in lead reaches back before the name.
        assert!(
            run.boxes[..(first_name - 13) as usize]
                .iter()
                .all(Vec::is_empty)
        );
    }
}

#[test]
fn every_name_lies_on_a_detected_plate() {
    for (b, background) in BACKGROUNDS.into_iter().enumerate() {
        let name = synth_name(&mut Rng::new(20 + b as u64), NameStyle::Latin(12));
        let (scene, spec) = banner_scene(&name, background);
        let run = run_banner(&scene, None);
        let mut plates = 0;
        for f in spec.settle()..spec.settle() + 40 {
            let seen = run.seen[f as usize];
            let (plate, name) = (seen.plate.unwrap(), seen.name.unwrap());
            assert!(plate.carries(&name.text) && name.plate == plate);
            // The plate is about 390 px wide and centred.
            assert!((plate.x0 + plate.x1 - 1920).abs() <= 8, "{plate:?}");
            assert!((380..=400).contains(&(plate.x1 - plate.x0)), "{plate:?}");
            plates += 1;
        }
        assert_eq!(plates, 40);
    }
}

#[test]
fn colour_filters_keep_the_banner() {
    // The banner is white and grey: hue and saturation filters leave it
    // alone, darkening and contrast filters move ticks, plate edges and
    // name strokes together.
    let filters = [
        Filter::Hue(120.0),
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
            brightness: -0.12,
            contrast: 0.85,
        },
        Filter::Levels {
            brightness: -0.3,
            contrast: 0.9,
        },
    ];
    for (f, filter) in filters.into_iter().enumerate() {
        let name = synth_name(&mut Rng::new(40 + f as u64), NameStyle::Latin(4 + 2 * f));
        let (scene, spec) = banner_scene(&name, BACKGROUNDS[f % 2]);
        let run = run_banner(&scene, Some(filter));
        let context = format!("{filter:?}");
        run.assert_clean(&context);
        // The name is found and settles: the settled box is the tracked
        // one, not a fallback.
        let settled = &run.boxes[spec.settle() as usize + 20];
        assert!(
            settled.len() == 1 && settled[0].conf == Confidence::Tracked,
            "{context}: {settled:?}"
        );
    }
}

#[test]
fn glare_over_the_settled_name_gets_the_fallback_box() {
    // Glare washes out the name from just before it centres to the end:
    // the episode never settles (port spec failure F4), the fallback box
    // covers it to the end. The last case is the widest possible name,
    // sixteen "W", which the fallback box must cover as well.
    let mut cases: Vec<(usize, Background, String)> = BACKGROUNDS
        .into_iter()
        .enumerate()
        .map(|(b, background)| {
            let name = synth_name(&mut Rng::new(60 + b as u64), NameStyle::Latin(9));
            (b, background, name)
        })
        .collect();
    cases.push((0, BACKGROUNDS[0], "W".repeat(16)));
    for (b, background, name) in cases {
        let spec = BannerSpec {
            hold: 60,
            ..BannerSpec::new("ACE", &name, 5)
        };
        let glare = spec.settle() - 3;
        let mut scene = Scene::new(
            Layout::reference(),
            background,
            Palette::DEFAULT,
            spec.end() + 20,
        )
        .with_banner(&FontSet::latin(), spec.clone());
        for start in glare..=spec.end() {
            scene = scene.with_flash(Flash {
                start,
                frames: 1,
                amount: 0.95,
                area: Some(RectI {
                    x0: 690,
                    y0: 298,
                    x1: 1240,
                    y1: 328,
                }),
            });
        }
        let run = run_banner(&scene, None);
        let context = format!("background {b}, {} letters", name.len());
        run.assert_clean(&context);
        let held: Vec<_> = (spec.settle() + 10..spec.settle() + 60)
            .map(|f| run.boxes[f as usize][0])
            .collect();
        assert!(
            held.iter().all(|h| h.conf == Confidence::WeakHold
                && h.rect.x0 <= 960 - FALLBACK_HALF
                && h.rect.x1 >= 960 + FALLBACK_HALF),
            "{context}: {:?}",
            held[0]
        );
    }
}
