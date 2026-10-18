//! The analysis job end to end on synthetic recordings: decode, crops,
//! every detector stage, the tracks, the project and the PTS table in the
//! cache, events, cancellation.

#![allow(clippy::unwrap_used, reason = "test code: an error is a test failure")]

mod synthetic;

use std::sync::Mutex;

use blurrd_core::{
    EncoderSetting, ErrorKind, JobEvent, JobId, Mode, Region, RegionSet, Stage, VerifyState,
    WarningCode,
};
use blurrd_engine::{
    AnalyzeRequest, CancellationToken, Engine, EngineConfig, LOOKAHEAD_MAX, OcrSetup, ProgressSink,
    cache,
};
use blurrd_testkit::Layout;
use blurrd_video::PtsTable;
use synthetic::Workdir;

/// Collects every event.
#[derive(Debug, Default)]
struct Events(Mutex<Vec<JobEvent>>);

impl ProgressSink for Events {
    fn emit(&self, event: JobEvent) {
        self.0.lock().unwrap().push(event);
    }
}

impl Events {
    fn take(&self) -> Vec<JobEvent> {
        std::mem::take(&mut self.0.lock().unwrap())
    }
}

/// Cancels its token after `after` progress events.
struct CancelAfter<'a> {
    token: &'a CancellationToken,
    after: usize,
    seen: Mutex<usize>,
}

impl ProgressSink for CancelAfter<'_> {
    fn emit(&self, event: JobEvent) {
        if let JobEvent::Progress { .. } = event {
            let mut seen = self.seen.lock().unwrap();
            *seen += 1;
            if *seen >= self.after {
                self.token.cancel();
            }
        }
    }
}

fn engine(work: &Workdir) -> Engine {
    let mut config = EngineConfig::new(work.path("cache"));
    config.gpu = false;
    config.proxy_encoder = EncoderSetting::Named(String::from("mpeg4"));
    Engine::new(config).unwrap()
}

fn request(input: std::path::PathBuf, regions: RegionSet) -> AnalyzeRequest {
    let mut req = AnalyzeRequest::new(JobId(uuid::Uuid::now_v7()), input);
    req.regions = regions;
    req.verify = false;
    req.previews = false;
    req
}

#[test]
fn analysis_covers_every_name_of_a_synthetic_recording() {
    let work = Workdir::new("analysis-1080p");
    let scene = synthetic::hud_scene(Layout::reference(), 240, 3);
    let clip = work.path("clip.mkv");
    synthetic::write_clip(&clip, &scene);
    let engine = engine(&work);
    let events = Events::default();
    let regions = RegionSet::all();
    let analysis = engine
        .analyze(
            &request(clip.clone(), regions),
            &events,
            &CancellationToken::new(),
        )
        .unwrap();

    let report = &analysis.report;
    assert_eq!(report.frames, 240);
    assert!(report.max_delay <= LOOKAHEAD_MAX, "{}", report.max_delay);
    assert_eq!(report.decoder, "software");
    let project = &analysis.project;
    assert_eq!(project.plan().validate(1920, 1080), Ok(()));
    assert_eq!(project.verify.state, VerifyState::Unverified);
    for region in [
        Region::KillFeed,
        Region::Banner,
        Region::VoiceChat,
        Region::TextChat,
    ] {
        assert!(
            project.tracks.iter().any(|t| t.region == region),
            "no {region:?} track"
        );
    }
    let summary = synthetic::evaluate(project, &scene, regions);
    assert!(summary.item_frames > 1000, "{summary:?}");
    assert_eq!(summary.leaked_item_frames, 0, "{summary:?}");
    let on_portraits = synthetic::boxes_on_portraits(project, &scene, regions, 0);
    assert!(on_portraits.is_empty(), "{on_portraits:#?}");

    // The cache holds the project and the PTS table of the video.
    assert_eq!(cache::read_project(&report.project_file).unwrap(), *project);
    let pts = PtsTable::read(&report.project_file.with_file_name("pts.bin")).unwrap();
    assert_eq!(pts.len(), 240);
    assert_eq!(cache::pts_sha256(&pts), project.media.pts_sha256);
    assert!(cache::same_file(
        &project.media.fingerprint,
        &cache::fingerprint(&clip).unwrap()
    ));

    // Events: started, stages, progress up to the last frame, finished;
    // without OCR the result is reported as unverified.
    let events = events.take();
    assert!(matches!(
        events.first(),
        Some(JobEvent::Stage {
            stage: Stage::Probe,
            ..
        })
    ));
    // Matroska has no frame count: the total is estimated from the
    // duration.
    assert!(events.iter().any(|e| matches!(
        e,
        JobEvent::Started { total_frames, .. } if total_frames.abs_diff(240) <= 1
    )));
    assert!(events.iter().any(|e| matches!(
        e,
        JobEvent::Progress {
            frame: 240,
            total: 240,
            ..
        }
    )));
    assert!(
        !events.iter().any(|e| matches!(
            e,
            JobEvent::Warning {
                code: WarningCode::OcrUnavailable,
                ..
            }
        )),
        "no OCR warning when verification is off"
    );
    let Some(JobEvent::Finished { summary, .. }) = events.last() else {
        panic!("{events:?}");
    };
    assert_eq!(summary.frames, 240);
    assert_eq!(summary.verify, Some(VerifyState::Unverified));
}

#[test]
fn analysis_maps_other_resolutions_onto_the_reference_hud() {
    // Frames are brought to the reference scale and the boxes mapped back,
    // with no code per resolution. Voice and text chat at sizes other
    // than 1080p, and the kill feed's portrait clearance below 1080p, depend
    // on the per-resolution calibration of M15, so this checks that the
    // kill feed and the banner leak nothing, and the clearance at 1440p.
    let checked: RegionSet = [Region::KillFeed, Region::Banner].into_iter().collect();
    for (k, (width, height)) in [(2560u32, 1440u32), (1280, 720)].into_iter().enumerate() {
        let work = Workdir::new(&format!("analysis-{height}p"));
        let scene = synthetic::hud_scene(Layout::wide(width, height), 150, 5 + k as u64);
        let clip = work.path("clip.mkv");
        synthetic::write_clip(&clip, &scene);
        let analysis = engine(&work)
            .analyze(
                &request(clip, RegionSet::all()),
                &Events::default(),
                &CancellationToken::new(),
            )
            .unwrap();
        let project = &analysis.project;
        assert_eq!(project.plan().validate(width, height), Ok(()));
        let summary = synthetic::evaluate(project, &scene, checked);
        let leaks: u64 = checked
            .iter()
            .map(|r| summary.leaks_per_region.get(&r).copied().unwrap_or(0))
            .sum();
        assert_eq!(leaks, 0, "{height}p: {summary:?}");
        if height > 1080 {
            let on_portraits = synthetic::boxes_on_portraits(project, &scene, checked, 1);
            assert!(on_portraits.is_empty(), "{height}p: {on_portraits:#?}");
        }
    }
}

#[test]
fn analysis_calibrates_other_aspect_ratios() {
    // The calibration finds the HUD in every layout: frames that are not
    // 16:9 keep their aspect ratio at the reference scale and the HUD
    // elements their edges (native 4:3 at 960 and at 1080 lines, 16:10,
    // 21:9); a 4:3 game stretched to fill a 16:9 frame widens the HUD, a
    // pillarboxed one moves it in by the bars. Every kill feed and banner
    // name is covered, and a layout the calibration found raises no
    // warning.
    let checked: RegionSet = [Region::KillFeed, Region::Banner].into_iter().collect();
    let layouts = [
        Layout::native_4x3(1280, 960),
        Layout::native_4x3(1440, 1080),
        Layout::wide_16x10(1920, 1200),
        Layout::wide(2560, 1080),
        Layout::stretched_4x3(1920, 1080),
        Layout::pillarbox(1920, 1080),
    ];
    for (k, layout) in layouts.into_iter().enumerate() {
        let (width, height) = (layout.width, layout.height);
        let work = Workdir::new(&format!("analysis-aspect-{k}"));
        let scene = synthetic::hud_scene(layout, 90, 21 + k as u64);
        let clip = work.path("clip.mkv");
        synthetic::write_clip(&clip, &scene);
        let events = Events::default();
        let analysis = engine(&work)
            .analyze(
                &request(clip, RegionSet::all()),
                &events,
                &CancellationToken::new(),
            )
            .unwrap_or_else(|e| panic!("{width}x{height}: {e:?}"));
        let project = &analysis.project;
        assert_eq!(project.plan().validate(width, height), Ok(()));
        assert!(
            project.geometry.confidence >= 0.5,
            "{width}x{height}: {:?}",
            project.geometry
        );
        assert_eq!(project.detector.mode, Mode::Tight, "{width}x{height}");
        let summary = synthetic::evaluate(project, &scene, checked);
        let leaks: u64 = checked
            .iter()
            .map(|r| summary.leaks_per_region.get(&r).copied().unwrap_or(0))
            .sum();
        assert!(summary.item_frames > 0, "{width}x{height}");
        assert_eq!(leaks, 0, "{width}x{height}: {summary:?}");
        let warned = events.take().iter().any(
            |e| matches!(e, JobEvent::Warning { code, .. } if *code == WarningCode::CalibrationLow),
        );
        assert!(!warned, "{width}x{height}");
    }
}

#[test]
fn analysis_without_hud_structure_is_safe_and_warned() {
    // Nothing on screen confirms where the HUD is: the analysis keeps the
    // layout of the frame size, draws Safe boxes and says so.
    let work = Workdir::new("analysis-uncalibrated");
    let scene = synthetic::empty_scene(Layout::stretched_4x3(1920, 1080), 60, 5);
    let clip = work.path("clip.mkv");
    synthetic::write_clip(&clip, &scene);
    let events = Events::default();
    let analysis = engine(&work)
        .analyze(
            &request(clip, RegionSet::all()),
            &events,
            &CancellationToken::new(),
        )
        .unwrap();
    assert!(analysis.project.geometry.confidence < 0.5);
    assert_eq!(analysis.project.detector.mode, Mode::Safe);
    let warned = events.take().iter().any(
        |e| matches!(e, JobEvent::Warning { code, .. } if *code == WarningCode::CalibrationLow),
    );
    assert!(warned);
}

#[test]
fn analysis_draws_safe_boxes_when_asked() {
    // The box fit of the request reaches the detectors: Safe covers every
    // name with its whole slot, so the boxes cover more than Tight ones.
    let work = Workdir::new("analysis-safe");
    let scene = synthetic::hud_scene(Layout::reference(), 120, 11);
    let clip = work.path("clip.mkv");
    synthetic::write_clip(&clip, &scene);
    let regions = RegionSet::all();
    let area = |mode: Mode| {
        let mut req = request(clip.clone(), regions);
        req.mode = mode;
        // A fresh cache per mode, so neither analysis sees the other.
        let analysis = engine(&Workdir::new(&format!("analysis-safe-{mode:?}")))
            .analyze(&req, &Events::default(), &CancellationToken::new())
            .unwrap();
        assert_eq!(analysis.project.detector.mode, mode);
        let summary = synthetic::evaluate(&analysis.project, &scene, regions);
        assert_eq!(summary.leaked_item_frames, 0, "{mode:?}: {summary:?}");
        let table = analysis.project.plan().frame_table(&regions);
        (0..table.frame_count())
            .flat_map(|f| table.boxes(f).iter().map(|b| b.rect.area()))
            .sum::<u64>()
    };
    let (tight, safe) = (area(Mode::Tight), area(Mode::Safe));
    assert!(tight > 0, "no boxes");
    assert!(
        safe > tight,
        "Safe {safe} px is not larger than Tight {tight} px"
    );
}

#[test]
fn analysis_runs_only_the_requested_regions_and_keeps_overrides() {
    let work = Workdir::new("analysis-regions");
    let scene = synthetic::hud_scene(Layout::reference(), 120, 7);
    let clip = work.path("clip.mkv");
    synthetic::write_clip(&clip, &scene);
    let engine = engine(&work);
    let regions: RegionSet = [Region::KillFeed].into_iter().collect();
    let first = engine
        .analyze(
            &request(clip.clone(), regions),
            &Events::default(),
            &CancellationToken::new(),
        )
        .unwrap();
    assert_ne!(first.project.tracks.len(), 0);
    assert!(
        first
            .project
            .tracks
            .iter()
            .all(|t| t.region == Region::KillFeed)
    );
    assert_eq!(first.project.detector.regions, regions);

    // A re-analysis replaces the tracks and keeps the overrides.
    let mut edited = first.project.clone();
    edited.overrides.push(blurrd_core::Override::Add {
        uid: uuid_like(),
        region: Region::Manual,
        f0: 1,
        f1: 5,
        rect: blurrd_core::RectI::new(10, 10, 50, 30).unwrap(),
    });
    cache::write_project(&edited, &first.report.project_file).unwrap();
    let second = engine
        .analyze(
            &request(clip, RegionSet::all()),
            &Events::default(),
            &CancellationToken::new(),
        )
        .unwrap();
    assert_eq!(second.project.overrides, edited.overrides);
    assert_eq!(second.project.revision, edited.revision + 1);
    assert!(
        second
            .project
            .tracks
            .iter()
            .any(|t| t.region == Region::TextChat)
    );
}

#[test]
fn a_reanalysis_rebinds_edits_by_overlap_and_warns_about_the_rest() {
    use blurrd_core::{BindStatus, Override, RectI};

    let work = Workdir::new("analysis-rebind");
    let scene = synthetic::hud_scene(Layout::reference(), 120, 11);
    let clip = work.path("clip.mkv");
    synthetic::write_clip(&clip, &scene);
    let engine = engine(&work);
    let kill_feed: RegionSet = [Region::KillFeed].into_iter().collect();
    let first = engine
        .analyze(
            &request(clip.clone(), kill_feed),
            &Events::default(),
            &CancellationToken::new(),
        )
        .unwrap();

    // Hide one kill feed name, nudged by 2 px as a drag would leave it;
    // move a box that never existed; add a manual box.
    let segment = first.project.tracks[0].segments[0];
    let frame = segment.f0;
    let r = segment.rect;
    let hidden = RectI::new(r.x0 + 2, r.y0, r.x1 + 2, r.y1).unwrap();
    let edits = vec![
        Override::Suppress {
            uid: uuid::Uuid::from_u128(1),
            region: Region::KillFeed,
            f0: frame,
            f1: segment.f1,
            anchor: hidden,
        },
        Override::Adjust {
            uid: uuid::Uuid::from_u128(2),
            region: Region::KillFeed,
            f0: 10,
            f1: 20,
            anchor: RectI::new(100, 900, 200, 930).unwrap(),
            rect: RectI::new(100, 900, 220, 940).unwrap(),
        },
        Override::Add {
            uid: uuid::Uuid::from_u128(3),
            region: Region::Manual,
            f0: frame,
            f1: frame + 10,
            rect: RectI::new(800, 400, 900, 450).unwrap(),
        },
    ];
    let mut edited = first.project.clone();
    edited.overrides.clone_from(&edits);
    cache::write_project(&edited, &first.report.project_file).unwrap();

    // The re-analysis replaces every track (now of all regions).
    let events = Events::default();
    let second = engine
        .analyze(
            &request(clip, RegionSet::all()),
            &events,
            &CancellationToken::new(),
        )
        .unwrap();
    let plan = second.project.plan();
    assert_eq!(second.project.overrides, edits);
    let status: Vec<BindStatus> = plan.bindings().iter().map(|b| b.status).collect();
    assert_eq!(
        status,
        [BindStatus::Bound, BindStatus::Unbound, BindStatus::Bound]
    );
    let unbound: Vec<Option<(u32, u32)>> = events
        .take()
        .into_iter()
        .filter_map(|e| match e {
            JobEvent::Warning {
                code: WarningCode::OverrideUnbound,
                frames,
                ..
            } => Some(frames),
            _ => None,
        })
        .collect();
    assert_eq!(unbound, [Some((10, 20))]);

    // On the frame, the hidden name has no box and the manual box is
    // there.
    let mut boxes = Vec::new();
    plan.boxes_at(frame, &RegionSet::all(), &mut boxes);
    assert!(boxes.iter().all(|b| b.rect.iou(&r) < 0.5), "{boxes:?}");
    assert!(
        boxes
            .iter()
            .any(|b| b.rect == RectI::new(800, 400, 900, 450).unwrap())
    );
}

#[test]
fn a_canceled_analysis_stops_and_writes_nothing() {
    let work = Workdir::new("analysis-cancel");
    let scene = synthetic::hud_scene(Layout::reference(), 240, 9);
    let clip = work.path("clip.mkv");
    synthetic::write_clip(&clip, &scene);
    let token = CancellationToken::new();
    let sink = CancelAfter {
        token: &token,
        after: 1,
        seen: Mutex::new(0),
    };
    // With the previews on: a canceled analysis writes no proxy either.
    let mut req = request(clip, RegionSet::all());
    req.previews = true;
    let error = engine(&work).analyze(&req, &sink, &token).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Canceled);
    let projects = work.path("cache").join("projects");
    let written: Vec<_> = std::fs::read_dir(&projects)
        .into_iter()
        .flatten()
        .flat_map(|d| std::fs::read_dir(d.unwrap().path()).unwrap())
        .map(|f| f.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains("project") || name.contains("proxy"))
        .collect();
    assert_eq!(written, Vec::<String>::new());
}

/// ONNX Runtime and the models from the environment, or `None` when they
/// are not installed (a failure when `BLURRD_REQUIRE_OCR` is set).
fn ocr_setup() -> Option<OcrSetup> {
    let required = std::env::var("BLURRD_REQUIRE_OCR").is_ok_and(|v| !v.is_empty() && v != "0");
    let path = |name: &str| std::env::var_os(name).map(std::path::PathBuf::from);
    match (path("ORT_DYLIB_PATH"), path("BLURRD_MODEL_DIR")) {
        (Some(dylib), Some(models)) if dylib.is_file() && models.is_dir() => {
            let mut setup = OcrSetup::new(dylib, models);
            setup.gpu = false;
            Some(setup)
        }
        _ => {
            assert!(!required, "run `cargo xtask fetch-ort` and `fetch-models`");
            eprintln!("skipped: ONNX Runtime or the OCR models are not installed");
            None
        }
    }
}

#[test]
fn verification_builds_the_name_dictionary_or_warns() {
    let work = Workdir::new("analysis-pass0");
    let scene = synthetic::hud_scene(Layout::reference(), 90, 11);
    let clip = work.path("clip.mkv");
    synthetic::write_clip(&clip, &scene);
    let mut req = request(clip.clone(), RegionSet::all());
    req.verify = true;

    // Without OCR the analysis runs and says the result is not verified.
    let events = Events::default();
    let analysis = engine(&work)
        .analyze(&req, &events, &CancellationToken::new())
        .unwrap();
    assert_eq!(analysis.report.pass0, None);
    assert!(events.take().iter().any(|e| matches!(
        e,
        JobEvent::Warning {
            code: WarningCode::OcrUnavailable,
            ..
        }
    )));

    // With OCR (on the CPU), pass 0 reads the protected zones.
    let Some(setup) = ocr_setup() else {
        return;
    };
    let mut config = EngineConfig::new(work.path("cache-ocr"));
    config.gpu = false;
    config.ocr = Some(setup);
    let analysis = Engine::new(config)
        .unwrap()
        .analyze(&req, &events, &CancellationToken::new())
        .unwrap();
    let stats = analysis.report.pass0.unwrap();
    assert!(
        stats.zone_crops_read > 0 && stats.name_reads > 0,
        "{stats:?}"
    );
    assert!(
        !events
            .take()
            .iter()
            .any(|e| matches!(e, JobEvent::Warning { .. }))
    );
}

#[test]
fn analysis_writes_thumbnails_and_a_preview_proxy() {
    let work = Workdir::new("analysis-previews");
    // 90 frames at 60 fps: thumbnails at 0, 0.5 and 1.0 s.
    let scene = synthetic::hud_scene(Layout::reference(), 90, 41);
    let clip = work.path("clip.mkv");
    synthetic::write_clip(&clip, &scene);
    let mut req = request(clip, RegionSet::all());
    req.previews = true;
    let events = Events::default();
    let engine = engine(&work);
    let analysis = engine
        .analyze(&req, &events, &CancellationToken::new())
        .unwrap();
    assert!(
        !events
            .take()
            .iter()
            .any(|e| matches!(e, JobEvent::Warning { .. }))
    );
    let report = &analysis.report;
    assert_eq!(report.thumbnails, 3);
    let dir = report.project_file.parent().unwrap();
    for k in 0..3 {
        let jpeg = std::fs::read(dir.join("thumbs").join(format!("{k:05}.jpg"))).unwrap();
        assert_eq!(&jpeg[..2], [0xFF, 0xD8], "a JPEG");
    }
    // Matroska is not playable in the desktop app's player: a proxy.
    let proxy = report.proxy.clone().unwrap();
    assert_eq!(proxy, dir.join("proxy.mp4"));
    let info = blurrd_video::probe(&proxy).unwrap();
    assert_eq!((info.width, info.height), (960, 540));
    let (count, _) = synthetic::frames_and_keyframes(&proxy);
    assert_eq!(count, 90);
    // A later analysis keeps the proxy.
    let modified = std::fs::metadata(&proxy).unwrap().modified().unwrap();
    let again = engine
        .analyze(&req, &Events::default(), &CancellationToken::new())
        .unwrap();
    assert_eq!(again.report.proxy.as_ref(), Some(&proxy));
    assert_eq!(
        std::fs::metadata(&proxy).unwrap().modified().unwrap(),
        modified
    );
}

#[test]
fn videos_too_small_to_blur_are_refused() {
    let work = Workdir::new("analysis-small");
    let scene = synthetic::hud_scene(Layout::wide(320, 180), 10, 43);
    let clip = work.path("clip.mkv");
    synthetic::write_clip(&clip, &scene);
    let error = engine(&work)
        .analyze(
            &request(clip, RegionSet::all()),
            &Events::default(),
            &CancellationToken::new(),
        )
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::UnsupportedMedia);
}

#[test]
fn a_missing_video_fails_with_not_found() {
    let work = Workdir::new("analysis-missing");
    let events = Events::default();
    let error = engine(&work)
        .analyze(
            &request(work.path("missing.mkv"), RegionSet::all()),
            &events,
            &CancellationToken::new(),
        )
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::NotFound);
    assert!(matches!(
        events.take().last(),
        Some(JobEvent::Failed { .. })
    ));
}

/// A fixed override id.
fn uuid_like() -> uuid::Uuid {
    uuid::Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef)
}
