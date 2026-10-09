//! OCR for blurrd's verification.
//!
//! blurrd checks its own output with OCR: names read on the source build a
//! dictionary, and the blurred crops are read again to prove that no name
//! is left. This crate provides the OCR itself:
//!
//! - [`OrtRuntime`] loads ONNX Runtime at run time (the ort crate's
//!   `load-dynamic`), so blurrd starts and renders without it and reports
//!   OCR as unavailable instead.
//! - [`ModelSet`] holds the PP-OCRv5 mobile detection and recognition
//!   models, checked against the SHA-256 values pinned in `models.toml`.
//! - [`OcrEngine`] creates their sessions with fixed input shapes on
//!   DirectML (the GPU picked by LUID) or the CPU and warms each one up.
//!
//! Recognised text stays in memory. Logs, errors and reports never contain
//! it (architecture plan 10).

#![allow(
    clippy::cast_possible_wrap,
    reason = "image sizes are at most a few thousand pixels, so coordinates never wrap in i32"
)]

mod engine;
mod error;
mod models;
mod rec;
mod runtime;

pub use engine::{Ep, GpuAdapter, OcrEngine, OcrOptions, ShapeStats, ZoneId, ZoneSpec};
pub use error::OcrError;
pub use models::{DET_MODEL, ModelSet, PinnedFile, REC_DICT, REC_MODEL};
pub use rec::{REC_HEIGHT, WIDTH_BUCKETS};
pub use runtime::OrtRuntime;
