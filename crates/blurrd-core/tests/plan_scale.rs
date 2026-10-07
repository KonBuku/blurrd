//! Size and scale checks of milestone M4 on synthetic plans.

mod support;

use blurrd_core::RegionSet;
use support::{PlanShape, synthetic_plan};

#[test]
fn ten_minute_plan_serialises_below_two_megabytes() {
    let plan = synthetic_plan(PlanShape::TEN_MINUTES);
    assert_eq!(plan.validate(1920, 1080), Ok(()));
    let json = serde_json::to_vec(&plan).unwrap();
    let segments: usize = plan.tracks.iter().map(|t| t.segments.len()).sum();
    println!(
        "10-minute plan: {} tracks, {segments} segments, {} overrides, {} bytes",
        plan.tracks.len(),
        plan.overrides.len(),
        json.len()
    );
    assert!(json.len() < 2 * 1024 * 1024, "{} bytes", json.len());
}

#[test]
fn minute_plan_builds_a_complete_frame_table() {
    let plan = synthetic_plan(PlanShape::MINUTE);
    assert_eq!(plan.validate(1920, 1080), Ok(()));
    let table = plan.frame_table(&RegionSet::all());
    assert_eq!(table.frame_count(), 3601);
    assert!(
        table.total_boxes() > 60_000,
        "{} boxes",
        table.total_boxes()
    );
}
