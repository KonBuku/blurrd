//! The audit of a verified output of a real recording from
//! `BLURRD_TEST_MEDIA` (local only, never in CI): it must agree with the
//! verification during the analysis. A regression test of the M17 verify
//! finding where the audit, reading without the analysis' HUD structure
//! signals, confirmed callouts, the buy menu and the scoreboard headers
//! as names and renamed a clean output `*_UNVERIFIED`. Only counts and
//! frame ranges are printed: no names, no text.
//!
//! OCR and the encoder use the GPU as the product does, so run it alone
//! on the machine:
//!
//! ```text
//! cargo nextest run -p blurrd-engine --release --run-ignored all real_audit --no-capture
//! ```

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "local measurement: a missing file or a failed job is a test failure"
)]

use std::path::PathBuf;

use blurrd_core::{JobEvent, JobId, VerifyState};
use blurrd_engine::{
    AnalyzeRequest, AuditRequest, CancellationToken, Engine, EngineConfig, OcrSetup, ProgressSink,
    RenderRequest,
};
use blurrd_testkit::media::{Manifest, MediaDir};

/// The recording of the finding (excerpt E17: a callout with a distance
/// marker, the buy menu and the scoreboard headers).
const CLIP: &str = "07.21-14.17";

/// Ignores events.
struct Quiet;

impl ProgressSink for Quiet {
    fn emit(&self, _: JobEvent) {}
}

fn job() -> JobId {
    JobId(uuid::Uuid::now_v7())
}

#[test]
#[ignore = "needs BLURRD_TEST_MEDIA, OCR and the GPU"]
fn real_audit_agrees_with_the_verification() {
    let Some(media) = MediaDir::from_env() else {
        eprintln!("skipped: BLURRD_TEST_MEDIA is not set");
        return;
    };
    let (Some(dylib), Some(models)) = (
        std::env::var_os("ORT_DYLIB_PATH"),
        std::env::var_os("BLURRD_MODEL_DIR"),
    ) else {
        eprintln!("skipped: ONNX Runtime or the OCR models are not installed");
        return;
    };
    let manifest = Manifest::load_repository().unwrap();
    let clip = media.clip_path(manifest.clip(CLIP).expect("the clip is in the manifest"));
    let work = std::env::temp_dir().join(format!("blurrd-real-audit-{}", std::process::id()));
    let mut config = EngineConfig::new(work.join("cache"));
    config.ocr = Some(OcrSetup::new(PathBuf::from(dylib), PathBuf::from(models)));
    let engine = Engine::new(config).unwrap();

    let mut req = AnalyzeRequest::new(job(), clip.clone());
    req.previews = false;
    let project = engine
        .analyze(&req, &Quiet, &CancellationToken::new())
        .unwrap()
        .project;
    assert_eq!(project.verify.state, VerifyState::Clean);
    let output = engine
        .render(
            &RenderRequest::new(job(), clip.clone(), project.clone(), work.join("out.mp4")),
            &Quiet,
            &CancellationToken::new(),
        )
        .unwrap()
        .output;
    let audit = engine
        .audit(
            &AuditRequest::new(job(), clip, output.clone(), project),
            &Quiet,
            &CancellationToken::new(),
        )
        .unwrap();
    let leaks: Vec<_> = audit
        .report
        .leaks
        .iter()
        .map(|l| (l.frame_range, l.region, l.kind, l.status))
        .collect();
    let _ = std::fs::remove_dir_all(&work);
    eprintln!("audit {:?}", audit.report.summary);
    assert_eq!(audit.report.summary.state, VerifyState::Clean, "{leaks:?}");
    assert_eq!((audit.output, audit.renamed), (output, false));
}
