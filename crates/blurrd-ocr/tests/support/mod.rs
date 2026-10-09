//! Shared helpers of the OCR integration tests: locating the runtime and
//! the models, and drawing text to read.

#![allow(
    dead_code,
    unreachable_pub,
    clippy::expect_used,
    clippy::cast_possible_wrap,
    reason = "test helpers shared through `mod support`: each test binary uses \
              a different subset, a panic is a test failure and test images \
              are small"
)]

use std::path::PathBuf;

use blurrd_ocr::{ModelSet, OrtRuntime};
use blurrd_testkit::FontSet;
use blurrd_testkit::font::TextStyle;

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

/// An RGB image of `width` × `height` pixels in a dark colour.
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl Canvas {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            data: [24u8, 28, 36].repeat((width * height) as usize),
        }
    }

    /// Draws `text` in near-white with its top-left ink at about `(x, y)`
    /// and capitals `cap_px` tall; returns the text's bounds.
    pub fn text(&mut self, fonts: &FontSet, text: &str, x: i32, y: i32, cap_px: f32) -> [i32; 4] {
        let bitmap = fonts.render(text, TextStyle::cap(cap_px));
        for by in 0..bitmap.height as i32 {
            for bx in 0..bitmap.width as i32 {
                let a = bitmap.at(bx, by);
                let (px, py) = (x + bx, y + by);
                if a <= 0.0
                    || px < 0
                    || py < 0
                    || px >= self.width as i32
                    || py >= self.height as i32
                {
                    continue;
                }
                let i = 3 * (py as usize * self.width as usize + px as usize);
                for c in 0..3 {
                    let bg = f32::from(self.data[i + c]);
                    self.data[i + c] = (bg + (245.0 - bg) * a).round() as u8;
                }
            }
        }
        [x, y, x + bitmap.width as i32, y + bitmap.height as i32]
    }
}

/// Case-folded alphanumeric characters of `text` (what name matching
/// compares).
pub fn fold(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}
