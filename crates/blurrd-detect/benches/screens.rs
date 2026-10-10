//! The screen-surface detectors on synthetic reference frames (plan
//! M13b budget: the gates of all four surfaces ≤ 0.3 ms per frame on one
//! core, the read of an active surface ≤ 1.0 ms). Run with
//! `cargo bench -p blurrd-detect --bench screens`.

#![allow(
    missing_docs,
    reason = "criterion generates the benchmark entry points"
)]
#![allow(clippy::expect_used, reason = "a benchmark stops on setup errors")]

use std::hint::black_box;

use blurrd_core::{Region, RegionSet};
use blurrd_detect::screens::{FrameView, ScreenMap};
use blurrd_detect::{RgbView, ScreenDetector};
use blurrd_testkit::hud::killfeed::Team;
use blurrd_testkit::hud::match_end::{MatchEndKind, MatchEndSpec};
use blurrd_testkit::hud::recap::RecapSpec;
use blurrd_testkit::hud::scoreboard::{ScoreRow, ScoreboardMode, ScoreboardSpec};
use blurrd_testkit::hud::screen::ScreenTiming;
use blurrd_testkit::hud::spectating::SpectatingSpec;
use blurrd_testkit::{
    Background, FontSet, Layout, NameStyle, Palette, RgbFrame, Rng, Scene, synth_name,
};
use criterion::{Criterion, criterion_group, criterion_main};

/// A reference frame showing every surface (they do not overlap) over
/// noise, or none of them.
fn frame(with_surfaces: bool) -> RgbFrame {
    let fonts = FontSet::latin();
    let mut rng = Rng::new(11);
    let timing = ScreenTiming::instant(0, 2);
    let mut names = |n: usize| -> Vec<String> {
        (0..n)
            .map(|_| synth_name(&mut rng, NameStyle::Latin(10)))
            .collect()
    };
    let scene = Scene::new(
        Layout::reference(),
        Background::noise(3),
        Palette::DEFAULT,
        2,
    );
    if !with_surfaces {
        return scene.render(0).frame;
    }
    let rows = names(10)
        .into_iter()
        .enumerate()
        .map(|(i, name)| ScoreRow {
            name,
            team: if i < 5 { Team::Ally } else { Team::Enemy },
            own: false,
            dead: false,
        })
        .collect();
    let killer = names(1).remove(0);
    let report = names(4);
    let spectated = names(1).remove(0);
    let cards = names(5);
    scene
        .with_scoreboard(
            &fonts,
            ScoreboardSpec {
                rows,
                mode: ScoreboardMode::Teams,
                timing,
            },
        )
        .with_recap(
            &fonts,
            RecapSpec {
                killer,
                killer_team: Team::Enemy,
                agent: "Agent".to_owned(),
                report,
                timeline: 1,
                timing,
            },
        )
        .with_spectating(
            &fonts,
            SpectatingSpec {
                name: spectated,
                timing,
            },
        )
        .with_match_end(
            &fonts,
            MatchEndSpec {
                kind: MatchEndKind::Victory,
                names: cards,
                mvp: 0,
                timing,
            },
        )
        .render(0)
        .frame
}

/// The frame scaled to a quarter (4 × 4 means), as the engine feeds the
/// gates.
fn quarter(frame: &RgbFrame) -> (Vec<u8>, u32, u32) {
    let (w, h) = (frame.width() / 4, frame.height() / 4);
    let src = frame.data();
    let stride = 3 * frame.width() as usize;
    let mut out = vec![0u8; (3 * w * h) as usize];
    for y in 0..h as usize {
        for x in 0..w as usize {
            for c in 0..3 {
                let mut sum = 0u32;
                for dy in 0..4 {
                    for dx in 0..4 {
                        sum += u32::from(src[(4 * y + dy) * stride + 3 * (4 * x + dx) + c]);
                    }
                }
                out[3 * (y * w as usize + x) + c] = ((sum + 8) / 16) as u8;
            }
        }
    }
    (out, w, h)
}

fn bench(c: &mut Criterion) {
    let map = ScreenMap::REFERENCE;
    for (label, with_surfaces) in [("gameplay", false), ("all surfaces", true)] {
        let frame = frame(with_surfaces);
        let (q, qw, qh) = quarter(&frame);
        let quarter = FrameView::scaled(
            RgbView::new(&q, qw, qh, 3 * qw as usize).expect("valid quarter frame"),
            0.25,
        );
        let mut detector = ScreenDetector::new(&RegionSet::all());
        c.bench_function(&format!("screen gates, {label}"), |b| {
            b.iter(|| detector.gate(black_box(&quarter), &map));
        });
        if !with_surfaces {
            continue;
        }
        let native = FrameView::native(
            RgbView::new(
                frame.data(),
                frame.width(),
                frame.height(),
                3 * frame.width() as usize,
            )
            .expect("valid frame"),
        );
        let signals = detector.gate(&quarter, &map);
        for (i, region) in Region::SCREENS.into_iter().enumerate() {
            let signal = signals[i];
            assert!(
                detector.read(region, &native, &map, &signal).is_some(),
                "{region:?} is found"
            );
            c.bench_function(&format!("screen read, {}", region.key()), |b| {
                b.iter(|| detector.read(region, black_box(&native), &map, &signal));
            });
        }
    }
}

criterion_group!(benches, bench);
criterion_main!(benches);
