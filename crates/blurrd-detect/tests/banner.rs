//! The banner detector and settle model on synthetic banners from
//! blurrd-testkit: every frame that shows a name is covered, from the
//! slide-in to the fade-out, and frames without a banner get no box.

#![allow(
    clippy::unwrap_used,
    reason = "test helpers on small frames: a panic is a failure"
)]

mod support;

use blurrd_core::Confidence;
use blurrd_testkit::hud::banner::BannerSpec;
use blurrd_testkit::{Background, FontSet, Layout, NameStyle, Palette, Rng, Scene, synth_name};

use support::run_banner;

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
