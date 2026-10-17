//! Verify and fix on synthetic recordings (plan 7.2 and milestone M17):
//! an analysis with OCR checks the blurred pixels of every name part,
//! reports the result without any text, and a clean streamed run gets
//! its own name (rendered again when the verification added fix boxes).
//! OCR runs on the CPU, so no GPU is needed.

#![allow(clippy::unwrap_used, reason = "test code: an error is a test failure")]

mod support;
mod synthetic;

use std::path::PathBuf;
use std::sync::Mutex;

use blurrd_core::{
    EncoderSetting, JobEvent, JobId, Region, Stage, TrackSource, VerifyReport, VerifyState, schema,
};
use blurrd_engine::{
    AnalyzeRequest, CancellationToken, Engine, EngineConfig, OcrSetup, ProgressSink, RunRequest,
};
use blurrd_ocr::fold;
use blurrd_testkit::Layout;
use synthetic::Workdir;

/// Collects every event.
#[derive(Debug, Default)]
struct Events(Mutex<Vec<JobEvent>>);

impl ProgressSink for Events {
    fn emit(&self, event: JobEvent) {
        self.0.lock().unwrap().push(event);
    }
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

#[test]
fn a_clean_analysis_is_verified_and_its_report_holds_no_text() {
    let work = Workdir::new("verify-clean");
    let Some(engine) = engine(&work) else {
        return;
    };
    let (scene, names) = synthetic::hud_scene_named(Layout::reference(), 150, 41);
    let clip = work.path("clip.mkv");
    synthetic::write_clip(&clip, &scene);
    let mut req = AnalyzeRequest::new(JobId(uuid::Uuid::now_v7()), clip);
    req.previews = false;
    let events = Events::default();
    let analysis = engine
        .analyze(&req, &events, &CancellationToken::new())
        .unwrap();

    let summary = analysis.report.verify.unwrap();
    assert_eq!(summary.state, VerifyState::Clean, "{summary:?}");
    assert_eq!((summary.open, summary.unverifiable_items), (0, 0));
    let project = &analysis.project;
    assert_eq!(project.verify.state, VerifyState::Clean);
    assert_eq!(project.verify.plan_revision, project.revision);
    // Every fixed leak added one fix box.
    let fixes = project
        .tracks
        .iter()
        .filter(|t| t.source == TrackSource::OcrFix)
        .count();
    assert_eq!(fixes, summary.fixed as usize);

    // The report is valid, counts the items of every region, and holds
    // no name, not even folded.
    let path = analysis.report.verify_report.unwrap();
    assert_eq!(
        path.file_name().and_then(|n| n.to_str()),
        project.verify.report.as_deref()
    );
    let json = std::fs::read_to_string(&path).unwrap();
    let report: VerifyReport = schema::from_json(&json).unwrap();
    assert_eq!(report.summary, summary);
    // The HUD zones are read at their layout events (each change and its
    // settle read 12 frames later) and at least every second (frames 0, 60
    // and 120). The scene and the detectors are deterministic, so the
    // schedule reads exactly these 19 of the 150 frames; any other number
    // means the read schedule changed.
    assert_eq!(report.frames_checked, 19);
    for region in [
        Region::KillFeed,
        Region::Banner,
        Region::VoiceChat,
        Region::TextChat,
    ] {
        let counts = report.per_region[&region];
        assert!(counts.items > 0, "{region:?}: {counts:?}");
        assert!(counts.positive_control >= 0.9, "{region:?}: {counts:?}");
    }
    let folded_json = fold(&json);
    for name in &names {
        let folded = fold(name);
        assert!(
            !json.contains(name.as_str()) && !folded_json.contains(&folded),
            "the report contains a name"
        );
    }

    let events = events.0.into_inner().unwrap();
    assert!(events.iter().any(|e| matches!(
        e,
        JobEvent::Stage {
            stage: Stage::Verify,
            ..
        }
    )));
    assert!(events.iter().any(|e| matches!(
        e,
        JobEvent::Finished { summary, .. } if summary.verify == Some(VerifyState::Clean)
    )));
}

#[test]
fn a_clean_streamed_run_gets_its_own_name() {
    let work = Workdir::new("verify-run");
    let Some(engine) = engine(&work) else {
        return;
    };
    let scene = synthetic::hud_scene(Layout::reference(), 120, 43);
    let clip = work.path("clip.mkv");
    synthetic::write_clip(&clip, &scene);
    let output = work.path("out.mkv");
    let mut req = RunRequest::new(JobId(uuid::Uuid::now_v7()), clip, output.clone());
    req.export.encoder = EncoderSetting::Named(String::from("ffv1"));
    let report = engine
        .run(&req, &Events::default(), &CancellationToken::new())
        .unwrap();
    assert_eq!(
        report.analysis.verify.map(|v| v.state),
        Some(VerifyState::Clean)
    );
    // Fix boxes need a second render; a clean result without them keeps
    // the streamed output under its own name.
    let fixed = report.analysis.verify.map_or(0, |v| v.fixed);
    assert_eq!(report.rerendered, fixed > 0);
    assert!(report.render.verified);
    assert_eq!(report.render.output, output);
    assert!(output.is_file());
    assert!(!work.path("out_UNVERIFIED.mkv").exists());
}
