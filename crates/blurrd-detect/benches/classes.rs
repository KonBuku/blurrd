//! Pixel classes on the kill-feed crop (1260 × 332, port spec 11.4).
//!
//! The crop is cut from a synthetic frame with six kill-feed rows and a
//! banner. Run with `cargo bench -p blurrd-detect`.

#![allow(
    missing_docs,
    reason = "criterion generates the benchmark entry points"
)]
#![allow(clippy::expect_used, reason = "a benchmark stops on setup errors")]

use std::hint::black_box;

use blurrd_detect::morph::{MorphScratch, white_tophat};
use blurrd_detect::{ClassParams, GrayImage, PixelClasses, RgbView};
use blurrd_testkit::hud::banner::BannerSpec;
use blurrd_testkit::hud::killfeed::{Kill, KillFeedSpec, Team, Weapon};
use blurrd_testkit::{Background, FontSet, Layout, NameStyle, Palette, Rng, Scene, synth_name};
use criterion::{Criterion, criterion_group, criterion_main};

/// The feed and banner crop of plan 5.4 (`x 660..1920, y 80..412`) of a
/// synthetic frame with six kill-feed rows and a banner.
fn feed_crop() -> (Vec<u8>, u32, u32) {
    let fonts = FontSet::latin();
    let mut rng = Rng::new(1);
    let kills = (0..6)
        .map(|i| Kill {
            killer: synth_name(&mut rng, NameStyle::Latin(9)),
            victim: synth_name(&mut rng, NameStyle::Latin(12)),
            killer_team: Team::Ally,
            victim_team: Team::Enemy,
            weapon: Weapon::Rifle,
            badge: None,
            own: false,
            headshot: i % 2 == 0,
            birth: i,
        })
        .collect();
    let scene = Scene::new(
        Layout::reference(),
        Background::noise(3),
        Palette::DEFAULT,
        60,
    )
    .with_kill_feed(&fonts, KillFeedSpec::new(kills))
    .with_banner(&fonts, BannerSpec::new("ACE", "Benchmark", 0));
    let fx = scene.render(40);
    let (x0, y0, w, h) = (660u32, 80u32, 1260u32, 332u32);
    let mut crop = Vec::with_capacity((3 * w * h) as usize);
    for y in y0..y0 + h {
        let start = (3 * (y * fx.frame.width() + x0)) as usize;
        crop.extend_from_slice(&fx.frame.data()[start..start + 3 * w as usize]);
    }
    (crop, w, h)
}

fn bench(c: &mut Criterion) {
    let (crop, w, h) = feed_crop();
    let view = RgbView::new(&crop, w, h, 3 * w as usize).expect("valid crop");
    let params = ClassParams::default();
    let mut classes = PixelClasses::new();
    c.bench_function("pixel classes 1260x332", |b| {
        b.iter(|| classes.compute(black_box(&view), &params));
    });
    let mn = classes.mn().clone();
    let mut out = GrayImage::default();
    let mut scratch = MorphScratch::default();
    c.bench_function("white top-hat 7x7 1260x332", |b| {
        b.iter(|| white_tophat(black_box(&mn), 7, 7, &mut out, &mut scratch));
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
