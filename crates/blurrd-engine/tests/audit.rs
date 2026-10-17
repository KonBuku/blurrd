//! The audit of rendered outputs on synthetic recordings (plan 7.2 step 8
//! and the M17 acceptance): it agrees with the verification during the
//! analysis on a clean output, finds the names of an output rendered
//! without some of its boxes and renames that output `*_UNVERIFIED`. OCR
//! runs on the CPU, so no GPU is needed.

#![allow(clippy::unwrap_used, reason = "test code: an error is a test failure")]

mod support;
mod synthetic;

use std::path::PathBuf;

use blurrd_core::{EncoderSetting, JobEvent, JobId, LeakStatus, Region, TrackSource, VerifyState};
use blurrd_engine::{
    AnalyzeRequest, AuditRequest, CancellationToken, Engine, EngineConfig, OcrSetup, ProgressSink,
    RenderRequest,
};
use blurrd_testkit::Layout;
use synthetic::Workdir;

/// Ignores every event.
struct Quiet;

impl ProgressSink for Quiet {
    fn emit(&self, _event: JobEvent) {}
}

/// An engine with OCR on the CPU and software decoding, or `None` when
/// OCR is not installed.
fn engine(work: &Workdir) -> Option<Engine> {
    support::ocr()?;
    let dylib = PathBuf::from(std::env::var_os("ORT_DYLIB_PATH")?);
    let models = PathBuf::from(std::env::var_os("BLURRD_MODEL_DIR")?);
    let mut setup = OcrSetup::new(dylib, models);
    setup.gpu = false;
    let mut config = EngineConfig::new(work.path("cache"));
    config.gpu = false;
    config.ocr = Some(setup);
    Some(Engine::new(config).unwrap())
}

fn job() -> JobId {
    JobId(uuid::Uuid::now_v7())
}

#[test]
fn the_audit_agrees_with_the_verification_and_flags_missing_boxes() {
    let work = Workdir::new("audit");
    let Some(engine) = engine(&work) else {
        return;
    };
    let scene = synthetic::hud_scene(Layout::reference(), 120, 59);
    let clip = work.path("clip.mkv");
    synthetic::write_clip(&clip, &scene);
    let mut req = AnalyzeRequest::new(job(), clip.clone());
    req.previews = false;
    let project = engine
        .analyze(&req, &Quiet, &CancellationToken::new())
        .unwrap()
        .project;
    let render = |project: &blurrd_core::Project, name: &str| {
        let mut req = RenderRequest::new(job(), clip.clone(), project.clone(), work.path(name));
        req.export.encoder = EncoderSetting::Named(String::from("ffv1"));
        req.accept_unverified = true;
        engine
            .render(&req, &Quiet, &CancellationToken::new())
            .unwrap()
            .output
    };

    // A clean, verified project: the audit of its output is clean too.
    assert_eq!(project.verify.state, VerifyState::Clean);
    let clean = render(&project, "clean.mkv");
    let audit = engine
        .audit(
            &AuditRequest::new(job(), clip.clone(), clean.clone(), project.clone()),
            &Quiet,
            &CancellationToken::new(),
        )
        .unwrap();
    assert_eq!(
        audit.report.summary.state,
        VerifyState::Clean,
        "{:?}",
        audit.report.leaks
    );
    assert_eq!((audit.output.clone(), audit.renamed), (clean, false));
    assert_eq!(
        audit.report.output_sha256.as_ref().map(String::len),
        Some(64)
    );
    assert!(audit.report_file.is_file());

    // Without every second kill-feed box the output shows those names.
    let mut stripped = project.clone();
    let mut feed_track = 0;
    stripped.tracks.retain(|t| {
        if t.region != Region::KillFeed || t.source != TrackSource::Detector {
            return true;
        }
        feed_track += 1;
        feed_track % 2 == 0
    });
    let leaky = render(&stripped, "leaky.mkv");
    let audit = engine
        .audit(
            &AuditRequest::new(job(), clip, leaky, stripped),
            &Quiet,
            &CancellationToken::new(),
        )
        .unwrap();
    assert_eq!(audit.report.summary.state, VerifyState::LeaksRemain);
    let feed = audit
        .report
        .leaks
        .iter()
        .filter(|l| l.region == Some(Region::KillFeed) && l.status == LeakStatus::Open)
        .count();
    assert!(feed >= 3, "{:?}", audit.report.leaks);
    assert!(audit.renamed);
    assert_eq!(audit.output, work.path("leaky_UNVERIFIED.mkv"));
    assert!(audit.output.is_file());
}
