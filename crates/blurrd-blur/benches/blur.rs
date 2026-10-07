//! Benchmarks of the CPU blur.
//!
//! Milestone M5 requires at most 0.2 ms per frame for 16 boxes of 160x30
//! on a 1080p frame, excluding the writable frame copy. Run with
//! `cargo bench -p blurrd-blur`.
//!
//! The `16 boxes 160x30` cases blur the same boxes on every frame, like a
//! kill feed that stays on screen: the backend keeps the masks and block
//! grids of box shapes it has seen. The `new shapes every frame` case is
//! the worst case, where every box has a size and grid offset that no
//! earlier frame had, so all of them are built in that frame.

#![allow(clippy::expect_used, reason = "a benchmark stops on setup errors")]

use std::hint::black_box;

use blurrd_blur::{BlurBackend, CpuBlur, PixelParams, YuvColour, prepare_boxes};
use blurrd_core::{BlurBox, BlurParams, BlurStyle, RectI, Region, Strength};
use criterion::{Criterion, criterion_group, criterion_main};

#[path = "../tests/support/mod.rs"]
mod support;

use support::{Format, OwnedFrame};

/// 16 boxes in two kill-feed-like columns; box `i` of frame `frame` is
/// `size(16 * frame + i)` pixels large with its grid origin at `offset`.
fn kill_feed(frame: i32, size: impl Fn(i32) -> (i32, i32, i32)) -> Vec<BlurBox> {
    (0..16)
        .map(|i| {
            let (x, y) = (1400 + (i % 2) * 260, 80 + (i / 2) * 41);
            let (w, h, offset) = size(16 * frame + i);
            BlurBox {
                rect: RectI::new(x, y, x + w, y + h).expect("non-empty box"),
                region: Region::KillFeed,
                seed: i as u32,
                grid_origin: (x + offset, y + 3),
                block_scale: 1.0,
            }
        })
        .collect()
}

fn blur(c: &mut Criterion) {
    let boxes = kill_feed(0, |_| (160, 30, 2));
    let cases = [
        ("pixel blur medium", Format::Nv12, BlurParams::default()),
        (
            "pixel blur medium p010",
            Format::P010,
            BlurParams::default(),
        ),
        (
            "pixel blur medium i420",
            Format::I420,
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

    // 1,024 frames of 16 boxes of 140..=180 x 26..=34 pixels with one of
    // four grid offsets: 1,476 shapes, so no frame finds its shapes stored
    // from the frames before.
    let frames: Vec<Vec<BlurBox>> = (0..1024)
        .map(|frame| {
            kill_feed(frame, |k| {
                (140 + k % 41, 26 + (k / 41) % 9, 2 + (k / 369) % 4)
            })
        })
        .collect();
    let pixel =
        PixelParams::new(&BlurParams::default(), 1.0, YuvColour::default()).expect("valid params");
    let mut frame = OwnedFrame::noise(Format::Nv12, 1920, 1080, 7);
    let mut prepared = Vec::with_capacity(16);
    let mut next = frames.iter().cycle();
    c.bench_function(
        "16 boxes about 160x30, new shapes every frame, pixel blur medium",
        |b| {
            b.iter(|| {
                let boxes = next.next().expect("an endless cycle");
                prepare_boxes(black_box(boxes), 1920, 1080, &pixel, &mut prepared);
                CpuBlur
                    .blur(&mut frame.view(), (0, 0), &prepared, &pixel)
                    .expect("boxes fit the frame");
            });
        },
    );
}

criterion_group!(benches, blur);
criterion_main!(benches);
