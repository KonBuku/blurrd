//! Benchmarks of the per-frame box table.
//!
//! Milestone M4 requires `frame_table` for a 3601-frame, 300-track plan in
//! less than 5 ms. Run with `cargo bench -p blurrd-core`.

use std::hint::black_box;

use blurrd_core::RegionSet;
use criterion::{Criterion, criterion_group, criterion_main};

#[path = "../tests/support/mod.rs"]
mod support;

use support::{PlanShape, synthetic_plan};

fn frame_table(c: &mut Criterion) {
    let plan = synthetic_plan(PlanShape::MINUTE);
    let enabled = RegionSet::all();
    c.bench_function("frame_table 3601 frames x 300 tracks", |b| {
        b.iter(|| black_box(&plan).frame_table(black_box(&enabled)));
    });
    let ten = synthetic_plan(PlanShape::TEN_MINUTES);
    c.bench_function("frame_table 36000 frames x 600 tracks", |b| {
        b.iter(|| black_box(&ten).frame_table(black_box(&enabled)));
    });
}

criterion_group!(benches, frame_table);
criterion_main!(benches);
