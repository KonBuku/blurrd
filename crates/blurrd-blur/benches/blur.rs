//! Benchmarks of the CPU blur.
//!
//! Milestone M5 requires at most 0.2 ms per frame for 16 boxes of 160x30
//! on a 1080p frame, excluding the writable frame copy. Run with
//! `cargo bench -p blurrd-blur`.

#![allow(clippy::expect_used, reason = "a benchmark stops on setup errors")]

use std::hint::black_box;

use blurrd_blur::{BlurBackend, CpuBlur, PixelParams, YuvColour, prepare_boxes};
use blurrd_core::{BlurBox, BlurParams, BlurStyle, RectI, Region, Strength};
use criterion::{Criterion, criterion_group, criterion_main};

#[path = "../tests/support/mod.rs"]
mod support;

use support::{Format, OwnedFrame};

/// 16 boxes of 160x30 in two kill-feed-like columns.
fn boxes() -> Vec<BlurBox> {
    (0..16)
        .map(|i| {
            let (x, y) = (1400 + (i % 2) * 260, 80 + (i / 2) * 41);
            BlurBox {
                rect: RectI::new(x, y, x + 160, y + 30).expect("non-empty box"),
                region: Region::KillFeed,
                seed: i as u32,
                grid_origin: (x + 2, y + 3),
                block_scale: 1.0,
            }
        })
        .collect()
}

fn blur(c: &mut Criterion) {
    let boxes = boxes();
    let cases = [
        ("pixel blur medium", Format::Nv12, BlurParams::default()),
        (
            "pixel blur medium p010",
            Format::P010,
            BlurParams::default(),
        ),
        (
            "pixel blur high",
            Format::Nv12,
            BlurParams {
                strength: Strength::High,
                ..BlurParams::default()
            },
        ),
        (
            "solid mean",
            Format::Nv12,
            BlurParams {
                style: BlurStyle::Solid {
                    color: blurrd_core::SolidColor::BoxMean,
                },
                ..BlurParams::default()
            },
        ),
        (
            "pixelate",
            Format::Nv12,
            BlurParams {
                style: BlurStyle::Pixelate,
                ..BlurParams::default()
            },
        ),
        (
            "gaussian medium",
            Format::Nv12,
            BlurParams {
                style: BlurStyle::Gaussian,
                ..BlurParams::default()
            },
        ),
    ];
    for (name, format, params) in cases {
        let pixel = PixelParams::new(&params, 1.0, YuvColour::default()).expect("valid params");
        let mut frame = OwnedFrame::noise(format, 1920, 1080, 7);
        let mut prepared = Vec::with_capacity(boxes.len());
        c.bench_function(&format!("16 boxes 160x30, {name}"), |b| {
            b.iter(|| {
                prepare_boxes(black_box(&boxes), 1920, 1080, &pixel, &mut prepared);
                CpuBlur
                    .blur(&mut frame.view(), (0, 0), &prepared, &pixel)
                    .expect("boxes fit the frame");
            });
        });
    }
}

criterion_group!(benches, blur);
criterion_main!(benches);
