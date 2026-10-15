//! Canceling the engine's jobs on a real recording from
//! `BLURRD_TEST_MEDIA` (local only, never in CI): the analysis, the
//! render and the streaming run each return within 100 ms of the cancel
//! while running at full speed, and leave no output or `.tmp` file. The
//! analysis and the run are canceled without and with OCR verification
//! (pass 0 on the GPU and on the CPU) when ONNX Runtime and the models
//! are installed (`ORT_DYLIB_PATH`, `BLURRD_MODEL_DIR`; required with
//! `BLURRD_REQUIRE_OCR`). Only times are printed.
//!
//! ```text
//! cargo nextest run -p blurrd-engine --release --run-ignored all real_cancel --no-capture
//! ```
//!
//! The render and the run encode with the default encoder chain (NVENC on
//! NVIDIA GPUs), as the app does.

#![allow(
    clippy::unwrap_used,
    reason = "local measurement: a missing file or a failed job is a test failure"
)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use blurrd_core::{AppError, ErrorKind, JobEvent, JobId};
use blurrd_engine::{
    AnalyzeRequest, CancellationToken, Engine, EngineConfig, OcrSetup, ProgressSink, RenderRequest,
    RunRequest,
};
use blurrd_testkit::media::{Manifest, MediaDir};

/// The longest time from the cancel to the job's return (plan M14).
const MAX_LATENCY: Duration = Duration::from_millis(100);
/// The job is canceled once it has done this many frames, well inside
/// the clip and at full speed.
const CANCEL_AT: u32 = 900;

/// Remembers the last progress frame.
#[derive(Default)]
struct Watch {
    frame: AtomicU32,
}

impl ProgressSink for Watch {
    fn emit(&self, event: JobEvent) {
        if let JobEvent::Progress { frame, .. } = event {
            self.frame.store(frame, Ordering::Release);
        }
    }
}

fn job() -> JobId {
    JobId(uuid::Uuid::now_v7())
}

/// Runs `job` on its own thread, cancels it at frame [`CANCEL_AT`] and
/// returns the time from the cancel to the job's return.
fn cancel_latency<T: Send>(
    name: &str,
    job: impl FnOnce(&Watch, &CancellationToken) -> Result<T, AppError> + Send,
) -> Duration {
    let watch = Watch::default();
    let token = CancellationToken::new();
    std::thread::scope(|scope| {
        let worker = scope.spawn(|| job(&watch, &token));
        while watch.frame.load(Ordering::Acquire) < CANCEL_AT {
            assert!(!worker.is_finished(), "{name} ended before the cancel");
            std::thread::sleep(Duration::from_millis(1));
        }
        token.cancel();
        let canceled = Instant::now();
        let result = worker.join().unwrap();
        let latency = canceled.elapsed();
        let error = result.err().map(|e| e.kind);
        assert_eq!(error, Some(ErrorKind::Canceled), "{name}");
        eprintln!(
            "{name}: returned {:.1} ms after the cancel",
            latency.as_secs_f64() * 1000.0
        );
        latency
    })
}

/// ONNX Runtime and the models from the environment, on the GPU or the
/// CPU, or `None` when they are not installed (a failure when
/// `BLURRD_REQUIRE_OCR` is set).
fn ocr_setup(gpu: bool) -> Option<OcrSetup> {
    let required = std::env::var("BLURRD_REQUIRE_OCR").is_ok_and(|v| !v.is_empty() && v != "0");
    let path = |name: &str| std::env::var_os(name).map(PathBuf::from);
    match (path("ORT_DYLIB_PATH"), path("BLURRD_MODEL_DIR")) {
        (Some(dylib), Some(models)) if dylib.is_file() && models.is_dir() => {
            let mut setup = OcrSetup::new(dylib, models);
            setup.gpu = gpu;
            Some(setup)
        }
        _ => {
            assert!(!required, "run `cargo xtask fetch-ort` and `fetch-models`");
            eprintln!("OCR cases skipped: ONNX Runtime or the OCR models are not installed");
            None
        }
    }
}

/// The files in `dir` whose name starts with `stem`.
fn leftovers(dir: &Path, stem: &str) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(stem))
        .collect()
}

#[test]
#[ignore = "needs BLURRD_TEST_MEDIA and a GPU encoder"]
fn real_jobs_cancel_quickly_and_leave_nothing() {
    let Some(media) = MediaDir::from_env() else {
        eprintln!("skipped: BLURRD_TEST_MEDIA is not set");
        return;
    };
    let manifest = Manifest::load_repository().unwrap();
    let clip = media.clip_path(&manifest.clips[0]);
    let work: PathBuf =
        std::env::temp_dir().join(format!("blurrd-real-cancel-{}", std::process::id()));
    std::fs::create_dir_all(&work).unwrap();
    let engine = Engine::new(EngineConfig::new(work.join("cache"))).unwrap();

    let analyze = |verify: bool| {
        let mut req = AnalyzeRequest::new(job(), clip.clone());
        req.verify = verify;
        req.previews = false;
        req
    };
    let mut latencies = vec![cancel_latency("analyze", |watch, token| {
        engine.analyze(&analyze(false), watch, token)
    })];

    let project = engine
        .analyze(
            &analyze(false),
            &Watch::default(),
            &CancellationToken::new(),
        )
        .unwrap()
        .project;
    latencies.push(cancel_latency("render", |watch, token| {
        let mut req = RenderRequest::new(job(), clip.clone(), project, work.join("render.mp4"));
        req.accept_unverified = true;
        engine.render(&req, watch, token)
    }));
    assert_eq!(leftovers(&work, "render"), Vec::<String>::new());

    latencies.push(cancel_latency("run", |watch, token| {
        let mut req = RunRequest::new(job(), clip.clone(), work.join("run.mp4"));
        req.verify = false;
        req.accept_unverified = true;
        engine.run(&req, watch, token)
    }));
    assert_eq!(leftovers(&work, "run"), Vec::<String>::new());

    // With OCR verification (pass 0 reads while the detectors run).
    for (gpu, ep) in [(true, "gpu"), (false, "cpu")] {
        let Some(setup) = ocr_setup(gpu) else {
            break;
        };
        let mut config = EngineConfig::new(work.join("cache"));
        config.ocr = Some(setup);
        let engine = Engine::new(config).unwrap();
        latencies.push(cancel_latency(
            &format!("analyze+ocr({ep})"),
            |watch, token| engine.analyze(&analyze(true), watch, token),
        ));
        latencies.push(cancel_latency(&format!("run+ocr({ep})"), |watch, token| {
            let mut req = RunRequest::new(job(), clip.clone(), work.join("run-ocr.mp4"));
            req.accept_unverified = true;
            engine.run(&req, watch, token)
        }));
        assert_eq!(leftovers(&work, "run-ocr"), Vec::<String>::new());
    }

    let _ = std::fs::remove_dir_all(&work);
    assert!(latencies.iter().all(|&l| l < MAX_LATENCY), "{latencies:?}");
}
