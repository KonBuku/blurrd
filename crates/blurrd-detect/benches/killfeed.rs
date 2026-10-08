//! The kill-feed detector on the feed crop of a synthetic frame (plan 11.4
//! budget: kill feed + banner ≤ 1.5 ms per frame, M9 gate for the kill
//! feed alone ≤ 1.2 ms). Run with `cargo bench -p blurrd-detect`.

#![allow(
    missing_docs,
    reason = "criterion generates the benchmark entry points"
)]
#![allow(clippy::expect_used, reason = "a benchmark stops on setup errors")]

use std::hint::black_box;

use blurrd_detect::killfeed::FEED_ROI;
use blurrd_detect::{KillFeedDetector, RgbView};
use blurrd_testkit::hud::killfeed::{Kill, KillFeedSpec, Team, Weapon};
use blurrd_testkit::{Background, FontSet, Layout, NameStyle, Palette, Rng, Scene, synth_name};
use criterion::{Criterion, criterion_group, criterion_main};

/// A reference frame with `rows` settled kill-feed rows.
fn frame(rows: u32) -> blurrd_testkit::RgbFrame {
    let fonts = FontSet::latin();
    let mut rng = Rng::new(5);
    let kills = (0..rows)
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
    Scene::new(
        Layout::reference(),
        Background::noise(3),
        Palette::DEFAULT,
        rows + 10,
    )
    .with_kill_feed(&fonts, KillFeedSpec::new(kills))
    .render(rows + 4)
    .frame
}

fn bench(c: &mut Criterion) {
    for rows in [0, 3, 6] {
        let frame = frame(rows);
        let stride = 3 * frame.width() as usize;
        let start = FEED_ROI.y0 as usize * stride + 3 * FEED_ROI.x0 as usize;
        let view = RgbView::new(
            &frame.data()[start..],
            FEED_ROI.width(),
            FEED_ROI.height(),
            stride,
        )
        .expect("valid crop");
        let mut detector = KillFeedDetector::new();
        let mut out = Vec::new();
        c.bench_function(&format!("kill feed detector, {rows} rows"), |b| {
            b.iter(|| detector.detect(black_box(&view), &mut out));
        });
    }
}

criterion_group!(benches, bench);
criterion_main!(benches);
