//! Shared types of blurrd.
//!
//! `blurrd-core` holds the data model that every other crate and the desktop
//! app agree on: regions, box tracks, blur plans, the versioned project,
//! export and settings schemas, events and errors. It performs no I/O and
//! contains no `unsafe` code, so it builds and tests without FFmpeg, ONNX
//! Runtime or a GPU.
//!
//! Coordinates are source pixels, frame numbers are presentation-order frame
//! indices, and frame ranges are inclusive (`f0..=f1`).

#![forbid(unsafe_code)]

mod geometry;
mod overrides;
mod plan;
mod region;

pub use geometry::{EmptyRectError, RectI};
pub use overrides::{ADJUST_MIN_IOU, BindStatus, Override, OverrideBinding, SUPPRESS_MIN_IOU};
pub use plan::{
    BlurBox, BlurPlan, Confidence, DensityLane, DensityLanes, FrameBoxTable, PlanError, Segment,
    Track, TrackId, TrackSource,
};
pub use region::{NameKind, Region, RegionSet};

/// The blurrd version, shared by every crate of the workspace.
///
/// It is written into project files and reports so that a file can be traced
/// back to the build that produced it.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
