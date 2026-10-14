//! The jobs of blurrd, shared by the CLI and the desktop app.
//!
//! The engine connects decoding, detection, OCR, blurring and encoding into
//! jobs. It grows with the milestones that need it; so far it holds:
//!
//! - [`Engine`]: the configuration and the jobs.
//! - [`Engine::analyze`]: the analysis job. A bounded frame pipeline
//!   decodes the video once, cuts the HUD crops on a fixed pool of
//!   workers, runs every region's detectors and temporal models in frame
//!   order and turns their boxes into the project's tracks; pass 0 of the
//!   verification reads the protected zones on an OCR worker. The project
//!   and the PTS table go into the [`cache`].
//! - [`dictionary`]: pass 0 of the verification, the in-memory name
//!   dictionary built by OCR over the protected zones of the source.
//!
//! Every job reports through a [`ProgressSink`] and stops at the next
//! frame once its [`CancellationToken`] fires.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_possible_wrap,
    reason = "frame sizes are at most a few thousand pixels, so coordinates never wrap in i32"
)]

mod analysis;
pub mod cache;
pub mod dictionary;
mod engine;
mod hud;
mod job;
mod resample;

pub use analysis::{Analysis, AnalyzeReport, AnalyzeRequest, LOOKAHEAD_MAX};
pub use dictionary::{DictionaryBuilder, Pass0Stats, ProtectedZone, ZoneKind, protected_zones};
pub use engine::{Engine, EngineConfig, OcrSetup, PoolSizes, default_cache_dir};
pub use job::{CancellationToken, ProgressSink};
