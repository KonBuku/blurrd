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
//!
//! Types that cross the IPC boundary derive [`ts_rs::TS`]; with the `schema`
//! feature, they also derive `schemars::JsonSchema`. The persisted documents
//! ([`Project`], [`Settings`], [`PresetFile`], [`VerifyReport`]) implement
//! [`Document`] and are read and written with [`schema::from_json`] and
//! [`schema::to_json`].

#![forbid(unsafe_code)]

mod blur_params;
mod capabilities;
mod error;
mod export;
mod geometry;
mod hud;
mod job;
mod media;
mod overrides;
mod plan;
mod project;
mod region;
pub mod schema;
mod settings;
mod time;
mod verify;

pub use blur_params::{BlurParams, BlurStyle, MIN_BLOCK_PX, SolidColor, Strength};
pub use capabilities::{
    Capabilities, DecoderInfo, EncoderInfo, GpuInfo, InvalidLuidError, Luid, OcrCapability,
    OcrProvider, ProbeState,
};
pub use error::{AppError, ErrorKind};
pub use export::{
    AudioMode, Codec, ConflictPolicy, Container, EncoderSetting, ExportSettings, KeyframePolicy,
    MAX_PRESET, MIN_PRESET, MetadataMode, MoovMode, NvencSessions, OutputFolder, OutputSettings,
    RateControl, Toggle,
};
pub use geometry::{EmptyRectError, RectI};
pub use hud::{HudGeometry, HudLayout, RegionCalibration};
pub use job::{JobEvent, JobId, JobKind, JobSummary, Stage, WarningCode};
pub use media::{AudioStreamInfo, ColorInfo, ColorRange, Fingerprint, HdrKind, MediaInfo};
pub use overrides::{ADJUST_MIN_IOU, BindStatus, Override, OverrideBinding, SUPPRESS_MIN_IOU};
pub use plan::{
    BlurBox, BlurPlan, Confidence, DensityLane, DensityLanes, FrameBoxTable, PlanError, Segment,
    Track, TrackId, TrackSource,
};
pub use project::{DetectorInfo, Project, ProjectMedia, ProjectVerify};
pub use region::{NameKind, Region, RegionSet};
pub use schema::{Document, SchemaError};
pub use settings::{
    Appearance, Audit, Coverage, Defaults, General, HwDecodeSetting, KeepNames, LogLevel, Mode,
    Preset, PresetFile, PreviewQuality, Privacy, Processing, ReduceMotion, RegionSettings,
    Sensitivity, Settings, TemporalParams, Theme, VerifyMode, VerifySettings,
};
pub use time::{FrameRange, Rational, ReversedRangeError, ZeroDenominatorError};
pub use verify::{
    Leak, LeakKind, LeakStatus, LeakZone, RegionVerify, VerifyReport, VerifyState, VerifySummary,
};

/// The blurrd version, shared by every crate of the workspace.
///
/// It is written into project files and reports so that a file can be traced
/// back to the build that produced it.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
