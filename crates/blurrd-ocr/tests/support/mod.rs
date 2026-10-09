//! Shared helpers of the OCR integration tests: locating the runtime and
//! the models.

#![allow(
    dead_code,
    unreachable_pub,
    clippy::expect_used,
    reason = "test helpers shared through `mod support`: each test binary uses \
              a different subset, and a panic is a test failure"
)]

use std::path::PathBuf;

use blurrd_ocr::{ModelSet, OrtRuntime};

/// Set to a non-empty value other than `0` to fail instead of skipping
/// when ONNX Runtime or the models are missing (CI sets it).
pub const REQUIRE_OCR_ENV: &str = "BLURRD_REQUIRE_OCR";

/// ONNX Runtime and the bundled models, or `None` (test skipped) when they
/// are not installed and [`REQUIRE_OCR_ENV`] is not set.
pub fn ocr() -> Option<(OrtRuntime, ModelSet)> {
    let dylib = std::env::var_os("ORT_DYLIB_PATH").map(PathBuf::from);
    let models = std::env::var_os("BLURRD_MODEL_DIR").map(PathBuf::from);
    let required = std::env::var(REQUIRE_OCR_ENV).is_ok_and(|v| !v.is_empty() && v != "0");
    let (Some(dylib), Some(models)) = (dylib, models) else {
        assert!(!required, "ORT_DYLIB_PATH and BLURRD_MODEL_DIR must be set");
        eprintln!("skipped: ORT_DYLIB_PATH or BLURRD_MODEL_DIR is not set");
        return None;
    };
    if !dylib.is_file() || !models.is_dir() {
        assert!(
            !required,
            "run `cargo xtask fetch-ort` and `cargo xtask fetch-models` first"
        );
        eprintln!("skipped: ONNX Runtime or the OCR models are not installed");
        return None;
    }
    let runtime = OrtRuntime::load(&dylib).expect("ONNX Runtime loads");
    let models = ModelSet::load(&models).expect("the bundled models load");
    Some((runtime, models))
}
