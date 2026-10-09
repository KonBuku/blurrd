//! The voice-chat detector on the left-panel crop of a synthetic frame
//! (plan 11.4 budget: voice + chat ≤ 0.5 ms per frame on one core). Run
//! with `cargo bench -p blurrd-detect --bench voice`.

#![allow(
    missing_docs,
    reason = "criterion generates the benchmark entry points"
)]
#![allow(clippy::expect_used, reason = "a benchmark stops on setup errors")]

use std::hint::black_box;

use blurrd_core::FrameRange;
use blurrd_detect::voice::LEFT_PANEL_ROI;
use blurrd_detect::{RgbView, VoiceDetector};
use blurrd_testkit::hud::voice::{self, Speaker, VoiceSpec};
use blurrd_testkit::{Background, FontSet, Layout, NameStyle, Palette, Rng, Scene, synth_name};
use criterion::{Criterion, criterion_group, criterion_main};

/// A reference frame with entries in the lowest `entries` slots.
fn frame(entries: u8) -> blurrd_testkit::RgbFrame {
    let fonts = FontSet::latin();
    let mut rng = Rng::new(5);
    let speakers = (0..entries)
        .map(|slot| Speaker {
            name: synth_name(&mut rng, NameStyle::Latin(10)),
            location: voice::LOCATIONS[usize::from(slot)].to_owned(),
            slot,
            shown: vec![FrameRange { f0: 0, f1: 1 }],
        })
        .collect();
    Scene::new(
        Layout::reference(),
        Background::noise(3),
        Palette::DEFAULT,
        2,
    )
    .with_voice(&fonts, VoiceSpec { speakers })
    .render(0)
    .frame
}

fn bench(c: &mut Criterion) {
    for entries in [0, 2, 4] {
        let frame = frame(entries);
        let stride = 3 * frame.width() as usize;
        let start = LEFT_PANEL_ROI.y0 as usize * stride + 3 * LEFT_PANEL_ROI.x0 as usize;
        let view = RgbView::new(
            &frame.data()[start..],
            LEFT_PANEL_ROI.width(),
            LEFT_PANEL_ROI.height(),
            stride,
        )
        .expect("valid crop");
        let mut detector = VoiceDetector::new();
        let mut out = Vec::new();
        c.bench_function(&format!("voice detector, {entries} entries"), |b| {
            b.iter(|| detector.detect(black_box(&view), &mut out));
        });
    }
}

criterion_group!(benches, bench);
criterion_main!(benches);
