//! The render job end to end on synthetic recordings: the boxes of the
//! analysed project are blurred and nothing else changes, outputs of
//! unverified projects are marked, videos that are not the project's are
//! refused, and canceling leaves nothing behind.

#![allow(
    clippy::unwrap_used,
    clippy::cast_possible_wrap,
    reason = "test code: an error is a test failure, frame sizes never wrap"
)]

mod synthetic;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use blurrd_blur::{PixelParams, PreparedBox, prepare_boxes};
use blurrd_core::{EncoderSetting, ErrorKind, JobEvent, JobId, Project, RectI, RegionSet};
use blurrd_engine::{
    AnalyzeRequest, CancellationToken, Engine, EngineConfig, ProgressSink, RenderRequest,
};
use blurrd_testkit::Layout;
use blurrd_video::{DecodeOptions, Decoder, VideoColour, YuvRoi};
use synthetic::Workdir;

/// Ignores events, or cancels a token on the first progress event.
struct Sink<'a>(Option<&'a CancellationToken>, Mutex<Vec<JobEvent>>);

impl ProgressSink for Sink<'_> {
    fn emit(&self, event: JobEvent) {
        if let (Some(token), JobEvent::Progress { .. }) = (self.0, &event) {
            token.cancel();
        }
        self.1.lock().unwrap().push(event);
    }
}

fn sink() -> Sink<'static> {
    Sink(None, Mutex::new(Vec::new()))
}

fn engine(work: &Workdir) -> Engine {
    let mut config = EngineConfig::new(work.path("cache"));
    config.gpu = false;
    Engine::new(config).unwrap()
}

/// A synthetic recording and its analysed project.
fn analysed(work: &Workdir, engine: &Engine, frames: u32, seed: u64) -> (PathBuf, Project) {
    let scene = synthetic::hud_scene(Layout::reference(), frames, seed);
    let clip = work.path(&format!("clip{seed}.mkv"));
    synthetic::write_clip(&clip, &scene);
    let mut req = AnalyzeRequest::new(JobId(uuid::Uuid::now_v7()), clip.clone());
    req.verify = false;
    req.previews = false;
    let analysis = engine
        .analyze(&req, &sink(), &CancellationToken::new())
        .unwrap();
    (clip, analysis.project)
}

/// A render of `clip` into `output` with the lossless test encoder.
fn request(clip: &Path, project: &Project, output: PathBuf) -> RenderRequest {
    let mut req = RenderRequest::new(
        JobId(uuid::Uuid::now_v7()),
        clip.to_path_buf(),
        project.clone(),
        output,
    );
    req.export.encoder = EncoderSetting::Named(String::from("ffv1"));
    req
}

/// The luma plane of every frame of `path`.
fn lumas(path: &Path) -> Vec<Vec<u8>> {
    let mut decoder = Decoder::open(path, &DecodeOptions::default()).unwrap();
    let mut roi = YuvRoi::new();
    let mut out = Vec::new();
    while let Some(frame) = decoder.next_frame().unwrap() {
        let rect = RectI::new(0, 0, frame.width() as i32, frame.height() as i32).unwrap();
        frame.roi_yuv(rect, &mut roi).unwrap();
        out.push(roi.luma().to_vec());
    }
    out
}

#[test]
fn renders_the_analysed_boxes_and_nothing_else() {
    let work = Workdir::new("render-boxes");
    let engine = engine(&work);
    let (clip, project) = analysed(&work, &engine, 120, 13);
    let events = sink();
    let report = engine
        .render(
            &request(&clip, &project, work.path("out.mkv")),
            &events,
            &CancellationToken::new(),
        )
        .unwrap();
    // Not verified (no OCR yet): the name says so.
    assert_eq!(report.output, work.path("out_UNVERIFIED.mkv"));
    assert!(!report.verified);
    assert_eq!(report.frames, 120);
    assert_eq!(report.encoder, "ffv1");
    let table = project.plan().frame_table(&RegionSet::all());
    let with_boxes = (0..120).filter(|&f| !table.boxes(f).is_empty()).count();
    assert_eq!(report.blurred_frames as usize, with_boxes);
    assert!(with_boxes > 60);
    assert!(matches!(
        events.1.lock().unwrap().last(),
        Some(JobEvent::Finished { .. })
    ));

    // Lossless both ways: pixels change only inside the padded boxes, and
    // inside them the names are gone.
    let info = blurrd_video::probe(&clip).unwrap();
    let colour = VideoColour::from_info(&info.color, info.height).yuv_colour();
    let params = PixelParams::new(&project.blur, project.geometry.sy, colour).unwrap();
    let (source, output) = (lumas(&clip), lumas(&report.output));
    assert_eq!(source.len(), output.len());
    let mut prepared: Vec<PreparedBox> = Vec::new();
    for (frame, (before, after)) in source.iter().zip(&output).enumerate() {
        prepare_boxes(
            table.boxes(frame as u32),
            1920,
            1080,
            &params,
            &mut prepared,
        );
        let inside = |i: usize| {
            let (x, y) = ((i % 1920) as i32, (i / 1920) as i32);
            prepared.iter().any(|b| {
                let p = b.padded();
                x >= p.x0 && x < p.x1 && y >= p.y0 && y < p.y1
            })
        };
        let changed: Vec<usize> = (0..before.len())
            .filter(|&i| before[i] != after[i])
            .collect();
        assert!(changed.iter().all(|&i| inside(i)), "frame {frame}");
        assert_eq!(changed.is_empty(), prepared.is_empty(), "frame {frame}");
    }
}

#[test]
fn accepted_renders_keep_their_name_and_other_videos_are_refused() {
    let work = Workdir::new("render-checks");
    let engine = engine(&work);
    let (clip, project) = analysed(&work, &engine, 60, 17);
    let mut req = request(&clip, &project, work.path("accepted.mkv"));
    req.accept_unverified = true;
    let report = engine
        .render(&req, &sink(), &CancellationToken::new())
        .unwrap();
    assert_eq!(report.output, work.path("accepted.mkv"));
    // Accepting an unverified project must not report it as verified;
    // the acceptance is reported on its own.
    assert!(report.accepted_unverified);
    assert!(!report.verified, "an accepted project is still unverified");

    // Another recording of the same length and timing is not the
    // project's video.
    let (other, _) = analysed(&work, &engine, 60, 19);
    let error = engine
        .render(
            &request(&other, &project, work.path("other.mkv")),
            &sink(),
            &CancellationToken::new(),
        )
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::ProjectMismatch);

    // Outputs never go into the cache folder.
    let error = engine
        .render(
            &request(&clip, &project, work.path("cache").join("x.mkv")),
            &sink(),
            &CancellationToken::new(),
        )
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::InvalidProject);
    assert!(!work.path("other.mkv").exists() && !work.path("other_UNVERIFIED.mkv").exists());
}

#[test]
fn a_canceled_render_leaves_no_file() {
    let work = Workdir::new("render-cancel");
    let engine = engine(&work);
    let (clip, project) = analysed(&work, &engine, 90, 23);
    let token = CancellationToken::new();
    let error = engine
        .render(
            &request(&clip, &project, work.path("out.mkv")),
            &Sink(Some(&token), Mutex::new(Vec::new())),
            &token,
        )
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Canceled);
    let left: Vec<_> = std::fs::read_dir(work.dir())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("out"))
        .collect();
    assert_eq!(left, Vec::<String>::new());
}
