//! The kill-feed detector on synthetic frames from blurrd-testkit: every
//! name of every settled row is found, and nothing else is.

#![allow(
    clippy::cast_possible_wrap,
    clippy::unwrap_used,
    reason = "test helpers on small frames: coordinates never wrap, a panic is a failure"
)]

mod support;

use blurrd_core::{NameKind, Region};
use blurrd_detect::KillFeedDetector;
use blurrd_testkit::hud::killfeed::{Kill, KillFeedSpec, Team, Weapon};
use blurrd_testkit::{Background, FontSet, Layout, NameStyle, Palette, Rng, Scene, synth_name};

use support::{Check, check_frame, feed_view};

/// Frames after its birth until a row has slid into place.
const SETTLED: u32 = 4;

/// A kill with every visual variant chosen from `rng`.
fn random_kill(rng: &mut Rng, birth: u32) -> Kill {
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

/// A scene with `rows` rows born one frame apart, rendered when all of
/// them have settled.
fn settled_feed(seed: u64, rows: u32, background: Background, palette: Palette) -> Check {
    let fonts = FontSet::latin();
    let mut rng = Rng::new(seed);
    let kills = (0..rows).map(|i| random_kill(&mut rng, i)).collect();
    let scene = Scene::new(Layout::reference(), background, palette, rows + SETTLED + 2)
        .with_kill_feed(&fonts, KillFeedSpec::new(kills));
    check_frame(&scene.render(rows - 1 + SETTLED))
}

#[test]
fn finds_every_settled_name() {
    let backgrounds = [
        Background::noise(3),
        Background::dusk(),
        Background::white_sky(),
    ];
    let mut names = 0;
    for seed in 0..24u64 {
        let rows = 1 + (seed % 8) as u32;
        let background = backgrounds[(seed % 3) as usize];
        let check = settled_feed(seed, rows, background, Palette::DEFAULT);
        check.assert_clean(&format!("seed {seed}, {rows} rows"));
        assert_eq!(check.rows, rows as usize, "seed {seed}: rows");
        names += check.items;
    }
    assert!(names >= 200, "{names} names checked");
}

#[test]
fn reports_rows_top_to_bottom_with_their_names() {
    let check = settled_feed(7, 5, Background::noise(1), Palette::DEFAULT);
    check.assert_found("five rows");
    let tops: Vec<i32> = check.row_tops.clone();
    assert_eq!(tops.len(), 5);
    for (i, top) in tops.iter().enumerate() {
        // The text-line anchor follows the tallest glyph at the right end
        // of the victim name, so it may sit a few pixels off the bar.
        let truth = 96 + 39 * i as i32;
        assert!((top - truth).abs() <= 3, "row {i}: {top} for {truth}");
    }
    for d in &check.detections {
        assert_eq!(d.region, Region::KillFeed);
        assert!(matches!(d.kind, NameKind::Killer | NameKind::Victim));
        let top = d.row_top.unwrap();
        assert!(tops.contains(&top));
        assert_eq!(d.rect.y0, top + 5);
        assert_eq!(d.rect.y1, (top + 28).max(d.text.y1 + 7));
        assert_eq!((d.rect.x0, d.rect.x1), (d.text.x0 - 5, d.text.x1 + 5));
    }
}

#[test]
fn an_empty_feed_has_no_rows_and_no_names() {
    let scene = Scene::new(
        Layout::reference(),
        Background::noise(9),
        Palette::DEFAULT,
        1,
    );
    let fx = scene.render(0);
    let mut detector = KillFeedDetector::new();
    let mut out = vec![];
    detector.detect(&feed_view(&fx.frame), &mut out).unwrap();
    assert!(out.is_empty() && detector.rows().is_empty());
}

#[test]
fn rejects_crops_of_another_size() {
    let fx = Scene::new(
        Layout::reference(),
        Background::noise(9),
        Palette::DEFAULT,
        1,
    )
    .render(0);
    let full = blurrd_detect::RgbView::new(
        fx.frame.data(),
        fx.frame.width(),
        fx.frame.height(),
        3 * fx.frame.width() as usize,
    )
    .unwrap();
    let err = KillFeedDetector::new()
        .detect(&full, &mut vec![])
        .unwrap_err();
    assert!(err.to_string().contains("1260 x 332"), "{err}");
}

#[test]
fn reusing_a_detector_gives_the_same_result() {
    let fonts = FontSet::latin();
    let mut rng = Rng::new(42);
    let kills = (0..6).map(|i| random_kill(&mut rng, i)).collect();
    let scene = Scene::new(
        Layout::reference(),
        Background::dusk(),
        Palette::DEFAULT,
        30,
    )
    .with_kill_feed(&fonts, KillFeedSpec::new(kills));
    let mut reused = KillFeedDetector::new();
    for frame in [12, 0, 25, 12, 3] {
        let fx = scene.render(frame);
        let view = feed_view(&fx.frame);
        let (mut a, mut b) = (vec![], vec![]);
        reused.detect(&view, &mut a).unwrap();
        KillFeedDetector::new().detect(&view, &mut b).unwrap();
        assert_eq!(a, b, "frame {frame}");
    }
}
