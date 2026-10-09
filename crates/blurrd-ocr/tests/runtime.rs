//! Loading ONNX Runtime and choosing the execution provider.

#![allow(
    clippy::cast_possible_wrap,
    clippy::unwrap_used,
    reason = "test helpers on small images: coordinates never wrap, a panic is a failure"
)]

mod support;

use blurrd_core::{Luid, OcrProvider};
use blurrd_ocr::{Ep, GpuAdapter, OcrEngine, OcrError, OcrOptions, ZoneId, ZoneSpec};

fn options(ep: Ep) -> OcrOptions {
    OcrOptions {
        ep,
        zones: vec![ZoneSpec::for_crop(ZoneId(0), 300, 90, 1.0)],
        ..OcrOptions::default()
    }
}

#[test]
fn runtime_reports_its_version() {
    let Some((runtime, _)) = support::ocr() else {
        return;
    };
    // The source commit of ONNX Runtime 1.24.4 (third_party/ort.toml).
    assert!(
        runtime.commit().starts_with("2d924974ef"),
        "{}",
        runtime.build_info()
    );
    assert!(runtime.has_directml(), "the pinned build contains DirectML");
    // Loading the same file again returns the loaded runtime.
    let again = blurrd_ocr::OrtRuntime::load(runtime.path()).unwrap();
    assert_eq!(again.commit(), runtime.commit());
}

#[test]
fn another_runtime_file_conflicts() {
    let Some((runtime, _)) = support::ocr() else {
        return;
    };
    let copy = std::env::temp_dir().join(format!("blurrd-ort-copy-{}.dll", std::process::id()));
    std::fs::copy(runtime.path(), &copy).unwrap();
    let error = blurrd_ocr::OrtRuntime::load(&copy).unwrap_err();
    assert!(matches!(error, OcrError::RuntimeConflict { .. }), "{error}");
    let _ = std::fs::remove_file(&copy);
}

#[test]
fn cpu_provider_builds_fixed_shape_sessions() {
    let Some((runtime, models)) = support::ocr() else {
        return;
    };
    let engine = OcrEngine::new(&runtime, &models, &options(Ep::Cpu)).unwrap();
    assert_eq!(engine.active_provider(), OcrProvider::Cpu);
    assert!(engine.fallback().is_none());
    let shapes: Vec<[usize; 4]> = engine.stats().iter().map(|(s, _)| *s).collect();
    assert_eq!(
        shapes,
        [
            [1, 3, 96, 320],
            [1, 3, 48, 160],
            [1, 3, 48, 320],
            [1, 3, 48, 640]
        ]
    );
    for (_, stats) in engine.stats() {
        assert!(stats.warmup_ms > 0.0);
        assert_eq!(stats.runs, 0);
        // The CPU compiles nothing per shape: no run counts as one.
        assert!(stats.slow_threshold_ms.is_infinite());
    }
}

/// When DirectML cannot be used (here: an adapter that does not exist),
/// `Auto` falls back to the CPU and says why; `DirectMl` fails.
#[test]
fn auto_falls_back_to_cpu_when_directml_fails() {
    let Some((runtime, models)) = support::ocr() else {
        return;
    };
    let mut broken = options(Ep::Auto);
    broken.adapter = Some(GpuAdapter {
        luid: Luid(0xdead_beef),
        dxgi_index: 99,
    });
    let engine = OcrEngine::new(&runtime, &models, &broken).unwrap();
    assert_eq!(engine.active_provider(), OcrProvider::Cpu);
    assert!(
        engine
            .fallback()
            .is_some_and(|reason| reason.contains("DirectML"))
    );

    broken.ep = Ep::DirectMl;
    let error = OcrEngine::new(&runtime, &models, &broken).unwrap_err();
    assert!(
        matches!(
            error,
            OcrError::ProviderUnavailable { .. } | OcrError::Session { .. }
        ),
        "{error}"
    );
}

#[test]
fn rejects_zone_sizes_that_are_not_multiples_of_32() {
    let Some((runtime, models)) = support::ocr() else {
        return;
    };
    let mut bad = options(Ep::Cpu);
    bad.zones[0].width = 300;
    let error = OcrEngine::new(&runtime, &models, &bad).unwrap_err();
    assert!(matches!(error, OcrError::InvalidInput(_)), "{error}");
    // Content larger than the input.
    bad.zones[0].width = 320;
    bad.zones[0].content_width = 321;
    let error = OcrEngine::new(&runtime, &models, &bad).unwrap_err();
    assert!(matches!(error, OcrError::InvalidInput(_)), "{error}");
}

/// DirectML on the high-performance GPU (local only: CI runners have no
/// GPU).
#[test]
#[ignore = "needs a DirectX 12 GPU"]
fn gpu_directml_is_active() {
    let Some((runtime, models)) = support::ocr() else {
        return;
    };
    let engine = OcrEngine::new(&runtime, &models, &options(Ep::Auto)).unwrap();
    assert_eq!(
        engine.active_provider(),
        OcrProvider::DirectMl,
        "{:?}",
        engine.fallback()
    );
    let explicit = OcrEngine::new(&runtime, &models, &options(Ep::DirectMl)).unwrap();
    assert_eq!(explicit.active_provider(), OcrProvider::DirectMl);
    // Every session is built for its fixed shape (batched recognition).
    let shapes: Vec<[usize; 4]> = explicit.stats().iter().map(|(s, _)| *s).collect();
    assert_eq!(
        shapes,
        [
            [1, 3, 96, 320],
            [16, 3, 48, 160],
            [8, 3, 48, 320],
            [4, 3, 48, 640]
        ]
    );
    // The stall threshold is creation plus warm-up, which a full graph
    // compilation on DirectML would repeat.
    for (shape, stats) in explicit.stats() {
        assert!(
            (stats.slow_threshold_ms - stats.create_ms - stats.warmup_ms).abs() < 1e-3,
            "{shape:?}: {stats:?}"
        );
    }
}
