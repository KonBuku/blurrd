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
//! - [`OcrEngine`] runs them with fixed input shapes on DirectML (the GPU
//!   picked by LUID) or the CPU: [`OcrEngine::detect`] finds text lines in
//!   a protected zone ([`TextMap`], with the DB post-processing written in
//!   this crate) and [`OcrEngine::recognize`] reads crops with CTC
//!   decoding.
//! - [`NameDictionary`] collects the names read on the source (in memory
//!   only), [`Whitelist`] lists the HUD words that are never names and
//!   [`CropDedup`] skips crops that did not change since their zone was
//!   last OCR'd.
//!
//! Recognised text stays in memory. Logs, errors and reports never contain
//! it (architecture plan 10).

#![allow(
    clippy::cast_possible_wrap,
    reason = "image sizes are at most a few thousand pixels, so coordinates never wrap in i32"
)]

mod dedup;
mod det;
mod dictionary;
mod engine;
mod error;
mod image;
mod models;
mod rec;
mod resample;
mod runtime;

pub use dedup::CropDedup;
pub use det::{BIN_THRESHOLD, BOX_THRESHOLD, ProbMap, TextBox, TextMap, UNCLIP_RATIO};
pub use dictionary::{NameDictionary, NameStats, Whitelist, fold, name_len};
pub use engine::{Ep, GpuAdapter, OcrEngine, OcrOptions, OcrRead, ShapeStats, ZoneId, ZoneSpec};
pub use error::OcrError;
pub use image::RgbView;
pub use models::{DET_MODEL, ModelSet, PinnedFile, REC_DICT, REC_MODEL};
pub use rec::{REC_HEIGHT, Recognition, WIDTH_BUCKETS};
pub use runtime::OrtRuntime;
