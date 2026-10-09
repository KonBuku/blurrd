//! Shared helpers of the engine's integration tests: the OCR runtime, the
//! positive-control rule and the colour filters of the robustness checks.

#![allow(
    dead_code,
    unreachable_pub,
    clippy::expect_used,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "test helpers shared through `mod support`: each test binary uses \
              a different subset, a panic is a test failure and filtered values \
              are clamped to 0..=255"
)]

use std::path::PathBuf;

use blurrd_ocr::{
    ModelSet, NameDictionary, OcrEngine, OrtRuntime, REC_HEIGHT, WIDTH_BUCKETS, ZoneSpec, fold,
};

/// ONNX Runtime and the bundled models, or `None` when they are not
/// installed (fails instead when `BLURRD_REQUIRE_OCR` is set, as in CI).
pub fn ocr() -> Option<(OrtRuntime, ModelSet)> {
    let required = std::env::var("BLURRD_REQUIRE_OCR").is_ok_and(|v| !v.is_empty() && v != "0");
    let dylib = std::env::var_os("ORT_DYLIB_PATH").map(PathBuf::from);
    let models = std::env::var_os("BLURRD_MODEL_DIR").map(PathBuf::from);
    match (dylib, models) {
        (Some(dylib), Some(models)) if dylib.is_file() && models.is_dir() => Some((
            OrtRuntime::load(&dylib).expect("ONNX Runtime loads"),
            ModelSet::load(&models).expect("the bundled models load"),
        )),
        _ => {
            assert!(!required, "run `cargo xtask fetch-ort` and `fetch-models`");
            eprintln!("skipped: ONNX Runtime or the OCR models are not installed");
            None
        }
    }
}

/// Checks the no-recompilation guarantee where it is made, at
/// construction: every detection session has exactly the input shape of
/// its zone (`zones` in any order) and every recognition session that of
/// its width bucket. Tensors are always built with the session's shape,
/// so every run uses it.
pub fn assert_fixed_shapes(engine: &OcrEngine, zones: &[ZoneSpec]) {
    let mut sorted = zones.to_vec();
    sorted.sort_by_key(|z| z.id);
    let shapes: Vec<[usize; 4]> = engine.stats().iter().map(|(s, _)| *s).collect();
    let (det, rec) = shapes.split_at(sorted.len().min(shapes.len()));
    let expected: Vec<[usize; 4]> = sorted
        .iter()
        .map(|z| [1, 3, z.height as usize, z.width as usize])
        .collect();
    assert_eq!(det, expected, "detection session shapes");
    let widths: Vec<usize> = rec.iter().map(|s| s[3]).collect();
    let expected: Vec<usize> = WIDTH_BUCKETS.iter().map(|&w| w as usize).collect();
    assert_eq!(widths, expected, "recognition session widths");
    for shape in rec {
        assert!(
            shape[0] >= 1 && shape[1..3] == [3, REC_HEIGHT as usize],
            "recognition session shape {shape:?}"
        );
    }
}

/// Slow runs are a stall metric of a shared GPU, not a recompilation
/// check: they stay within [`blurrd_ocr::ShapeStats::stalls_tolerable`].
pub fn assert_stalls_tolerable(engine: &OcrEngine) {
    for (shape, stats) in engine.stats() {
        assert!(stats.stalls_tolerable(), "{shape:?}: {stats:?}");
    }
}

/// Whether OCR read `name`: a dictionary entry equals it up to the glyphs
/// OCR confuses (`0`/`o`, `1`/`l`/`i`, which the blurred output would show
/// the same way), or, for names of five characters or more, shares a run
/// of at least `max(4, 0.7 × length)` such characters with it (the rule
/// that matches a leak of the name in verification, plan 5.5).
pub fn was_read(dict: &NameDictionary, name: &str) -> bool {
    let truth = skeleton(&fold(name));
    let needed = if truth.len() < 5 {
        truth.len()
    } else {
        4.max((0.7 * truth.len() as f32).ceil() as usize)
    };
    dict.iter().any(|(entry, _)| {
        let entry = skeleton(entry);
        entry == truth || (truth.len() >= 5 && longest_common_run(&truth, &entry) >= needed)
    })
}

/// The folded name with confusable glyphs merged.
pub fn skeleton(folded: &str) -> Vec<char> {
    folded
        .chars()
        .map(|c| match c {
            '0' => 'o',
            '1' | 'i' => 'l',
            other => other,
        })
        .collect()
}

/// Length of the longest common substring of two character sequences.
pub fn longest_common_run(a: &[char], b: &[char]) -> usize {
    let mut best = 0;
    let mut previous = vec![0usize; b.len() + 1];
    for &ca in a {
        let mut current = vec![0usize; b.len() + 1];
        for (j, &cb) in b.iter().enumerate() {
            if ca == cb {
                current[j + 1] = previous[j] + 1;
                best = best.max(current[j + 1]);
            }
        }
        previous = current;
    }
    best
}

/// A colour change applied to a whole frame: a GPU driver filter (NVIDIA
/// Freestyle / Digital Vibrance, AMD colour settings) or an edited video.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Filter {
    /// Rotates every hue by this many degrees (luminance-preserving
    /// matrix, as CSS `hue-rotate`).
    Hue(f32),
    /// Scales the saturation: 0 is grey, 1 unchanged.
    Saturation(f32),
    /// Boosts dull colours more than vivid ones; 0 is unchanged.
    Vibrance(f32),
    /// `255 * (c / 255)^gamma`.
    Gamma(f32),
    /// Adds `brightness * 255`, then scales the distance from mid-grey by
    /// `contrast`.
    Levels {
        /// Brightness offset (-1 to 1).
        brightness: f32,
        /// Contrast factor.
        contrast: f32,
    },
}

impl Filter {
    /// Filters packed RGB pixels in place.
    pub fn apply(self, data: &mut [u8]) {
        for pixel in data.as_chunks_mut::<3>().0 {
            *pixel = self.pixel(*pixel);
        }
    }

    fn pixel(self, rgb: [u8; 3]) -> [u8; 3] {
        let c = rgb.map(f32::from);
        let luma = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        let out = match self {
            Self::Hue(degrees) => {
                let (sin, cos) = degrees.to_radians().sin_cos();
                let m = [
                    [
                        0.213 + cos * 0.787 - sin * 0.213,
                        0.715 - cos * 0.715 - sin * 0.715,
                        0.072 - cos * 0.072 + sin * 0.928,
                    ],
                    [
                        0.213 - cos * 0.213 + sin * 0.143,
                        0.715 + cos * 0.285 + sin * 0.140,
                        0.072 - cos * 0.072 - sin * 0.283,
                    ],
                    [
                        0.213 - cos * 0.213 - sin * 0.787,
                        0.715 - cos * 0.715 + sin * 0.715,
                        0.072 + cos * 0.928 + sin * 0.072,
                    ],
                ];
                m.map(|row| row[0] * c[0] + row[1] * c[1] + row[2] * c[2])
            }
            Self::Saturation(s) => c.map(|v| luma + s * (v - luma)),
            Self::Vibrance(amount) => {
                let (mx, mn) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
                let saturation = if mx > 0.0 { (mx - mn) / mx } else { 0.0 };
                let s = 1.0 + amount * (1.0 - saturation);
                c.map(|v| luma + s * (v - luma))
            }
            Self::Gamma(g) => c.map(|v| 255.0 * (v / 255.0).powf(g)),
            Self::Levels {
                brightness,
                contrast,
            } => c.map(|v| (v + brightness * 255.0 - 128.0) * contrast + 128.0),
        };
        out.map(|v| v.round().clamp(0.0, 255.0) as u8)
    }
}
