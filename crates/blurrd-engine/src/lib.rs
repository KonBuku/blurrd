//! The jobs of blurrd, shared by the CLI and the desktop app.
//!
//! The engine connects decoding, detection, OCR, blurring and encoding into
//! jobs. It grows with the milestones that need it; so far it holds:
//!
//! - [`Engine`]: the configuration and the jobs.
//! - [`Engine::analyze`]: the analysis job. It first calibrates the HUD
//!   geometry (and an HDR video's paper white) from keyframes sampled
//!   over the video. A bounded frame pipeline then decodes the video
//!   once, cuts the HUD crops on a fixed pool of workers, runs every
//!   region's detectors and temporal models in frame order and turns
//!   their boxes into the project's tracks; pass 0 of the
//!   verification reads the protected zones on an OCR worker, and a
//!   preview worker writes thumbnails and, for videos the desktop app's
//!   player cannot play, a preview proxy. The project, the PTS table and
//!   the previews go into the [`cache`].
//! - [`Engine::render`]: the render job. It checks that the video is the
//!   project's (fingerprint, timestamps from the cached PTS table,
//!   keyframes), blurs every frame's boxes and encodes the export, named
//!   `*_UNVERIFIED` while the project is not verified.
//! - [`Engine::run`]: analysis and render in one streaming job. The render
//!   decoder trails the analysis decoder and blurs each frame once its
//!   boxes are final, so the output equals an analysis followed by a
//!   render.
//! - [`Engine::reverify_ranges`]: the verification of the frame ranges
//!   edits touched, after the analysis.
//! - [`Engine::audit`]: OCR over a rendered output, as an end-to-end
//!   check of what the encoder wrote; an output with a readable name is
//!   renamed `*_UNVERIFIED`.
//! - [`dictionary`]: pass 0 of the verification, the in-memory name
//!   dictionary built by OCR over the protected zones of the source.
//! - The verification itself: after the detectors, the analysis reads
//!   the blurred pixels of every name part the boxes do not cover,
//!   adds `OcrFix` slot boxes where a name stays readable and saves a
//!   report (`verify-<revision>.json`) next to the project.
//!
//! Every job reports through a [`ProgressSink`] and stops at the next
//! frame once its [`CancellationToken`] fires.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_possible_wrap,
    reason = "frame sizes are at most a few thousand pixels, so coordinates never wrap in i32"
)]

mod analysis;
mod audit;
pub mod cache;
pub mod dictionary;
mod edits;
mod engine;
mod hud;
mod job;
mod render;
mod resample;
mod reverify;
mod run;
mod scaler;
mod slots;
mod verify;

pub use analysis::{Analysis, AnalyzeReport, AnalyzeRequest, LOOKAHEAD_MAX};
pub use audit::{AuditReport, AuditRequest};
pub use dictionary::{
    DictionaryBuilder, NamePart, Pass0Stats, ProtectedZone, SourceLine, ZoneKind, full_frame_zone,
    protected_zones, screen_zones,
};
pub use engine::{Engine, EngineConfig, OcrSetup, PoolSizes, default_cache_dir};
pub use job::{CancellationToken, ProgressSink};
pub use render::{
    FallbackReport, RenderReport, RenderRequest, SkippedReport, UNVERIFIED_SUFFIX, is_verified,
    unverified_name,
};
pub use reverify::{Reverify, ReverifyRequest, ReverifySummary};
pub use run::{RunReport, RunRequest};
