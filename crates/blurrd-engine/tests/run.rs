//! The streaming run end to end on synthetic recordings: its output is
//! identical to an analysis followed by a render, its project is the
//! analysis' project, cached overrides switch it to analyse-then-render,
//! and canceling leaves nothing behind.

#![allow(clippy::unwrap_used, reason = "test code: an error is a test failure")]

mod synthetic;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use blurrd_core::{EncoderSetting, ErrorKind, JobEvent, JobId, Override, RectI, Region};
use blurrd_engine::{
    AnalyzeRequest, CancellationToken, Engine, EngineConfig, ProgressSink, RenderRequest,
    RunRequest, cache,
};
use blurrd_testkit::Layout;
use synthetic::Workdir;

/// Collects events; cancels a token after `cancel_after` progress events.
struct Sink<'a> {
    cancel: Option<(&'a CancellationToken, usize)>,
    events: Mutex<Vec<JobEvent>>,
}

impl ProgressSink for Sink<'_> {
    fn emit(&self, event: JobEvent) {
        let mut events = self.events.lock().unwrap();
        events.push(event);
        if let Some((token, after)) = self.cancel {
            let progress = events
                .iter()
                .filter(|e| matches!(e, JobEvent::Progress { .. }))
                .count();
            if progress >= after {
                token.cancel();
            }
        }
    }
}

fn sink() -> Sink<'static> {
    Sink {
        cancel: None,
        events: Mutex::new(Vec::new()),
    }
}

fn engine(cache: PathBuf) -> Engine {
    let mut config = EngineConfig::new(cache);
    config.gpu = false;
    Engine::new(config).unwrap()
}

fn job() -> JobId {
    JobId(uuid::Uuid::now_v7())
}

fn run_request(clip: &Path, output: PathBuf) -> RunRequest {
    let mut req = RunRequest::new(job(), clip.to_path_buf(), output);
    req.export.encoder = EncoderSetting::Named(String::from("ffv1"));
    req.accept_unverified = true;
    req.verify = false;
    req
}

#[test]
fn a_streamed_run_equals_analysis_then_render() {
    let work = Workdir::new("run-identical");
    let scene = synthetic::hud_scene(Layout::reference(), 240, 29);
    let clip = work.path("clip.mkv");
    synthetic::write_clip(&clip, &scene);

    let events = sink();
    let streamed = engine(work.path("cache-run"))
        .run(
            &run_request(&clip, work.path("run.mkv")),
            &events,
            &CancellationToken::new(),
        )
        .unwrap();
    assert!(streamed.streamed);
    assert_eq!(streamed.render.output, work.path("run.mkv"));
    assert_eq!(streamed.render.frames, 240);
    assert_eq!(streamed.analysis.frames, 240);
    let events = events.events.into_inner().unwrap();
    assert!(events.iter().any(|e| matches!(
        e,
        JobEvent::Started {
            total_frames: 240,
            ..
        }
    )));
    assert!(matches!(events.last(), Some(JobEvent::Finished { .. })));

    let two_pass = engine(work.path("cache-two-pass"));
    let mut analyze = AnalyzeRequest::new(job(), clip.clone());
    analyze.verify = false;
    let analysis = two_pass
        .analyze(&analyze, &sink(), &CancellationToken::new())
        .unwrap();
    let mut render = RenderRequest::new(
        job(),
        clip.clone(),
        analysis.project.clone(),
        work.path("two-pass.mkv"),
    );
    render.export.encoder = EncoderSetting::Named(String::from("ffv1"));
    render.accept_unverified = true;
    let rendered = two_pass
        .render(&render, &sink(), &CancellationToken::new())
        .unwrap();
    assert_eq!(rendered.blurred_frames, streamed.render.blurred_frames);
    assert!(rendered.blurred_frames > 100);

    // The same project and the same video stream, packet for packet
    // (Matroska headers differ only in their random segment id, date and
    // checksums).
    let saved = cache::read_project(&streamed.analysis.project_file).unwrap();
    assert_eq!(saved.tracks, analysis.project.tracks);
    assert_eq!(saved.media, analysis.project.media);
    assert_eq!(
        synthetic::video_stream_digest(&streamed.render.output),
        synthetic::video_stream_digest(&rendered.output)
    );
}

#[test]
fn cached_overrides_make_the_run_analyse_first() {
    let work = Workdir::new("run-overrides");
    let scene = synthetic::hud_scene(Layout::reference(), 90, 31);
    let clip = work.path("clip.mkv");
    synthetic::write_clip(&clip, &scene);
    let engine = engine(work.path("cache"));
    let mut analyze = AnalyzeRequest::new(job(), clip.clone());
    analyze.verify = false;
    let analysis = engine
        .analyze(&analyze, &sink(), &CancellationToken::new())
        .unwrap();
    let mut project = analysis.project;
    project.overrides.push(Override::Add {
        uid: uuid::Uuid::from_u128(5),
        region: Region::Manual,
        f0: 0,
        f1: 89,
        rect: RectI::new(800, 400, 900, 450).unwrap(),
    });
    cache::write_project(&project, &analysis.report.project_file).unwrap();

    let report = engine
        .run(
            &run_request(&clip, work.path("run.mkv")),
            &sink(),
            &CancellationToken::new(),
        )
        .unwrap();
    assert!(!report.streamed);
    // The manual box is on every frame.
    assert_eq!(report.render.blurred_frames, 90);
    let saved = cache::read_project(&report.analysis.project_file).unwrap();
    assert_eq!(saved.overrides, project.overrides);
}

#[test]
fn a_canceled_run_leaves_no_output() {
    let work = Workdir::new("run-cancel");
    let scene = synthetic::hud_scene(Layout::reference(), 240, 37);
    let clip = work.path("clip.mkv");
    synthetic::write_clip(&clip, &scene);
    let token = CancellationToken::new();
    let events = Sink {
        cancel: Some((&token, 3)),
        events: Mutex::new(Vec::new()),
    };
    let error = engine(work.path("cache"))
        .run(&run_request(&clip, work.path("run.mkv")), &events, &token)
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Canceled);
    assert!(matches!(
        events.events.lock().unwrap().last(),
        Some(JobEvent::Canceled { .. })
    ));
    let left: Vec<_> = std::fs::read_dir(work.dir())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("run"))
        .collect();
    assert_eq!(left, Vec::<String>::new());
}
