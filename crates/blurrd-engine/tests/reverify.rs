//! Verifying edited frame ranges again on synthetic recordings (plan 7.2
//! step 7 and the M17 acceptance): with part of the detector's boxes
//! removed, the verification finds every name left readable and covers
//! it, also a name it can only find without the dictionary. OCR runs on
//! the CPU, so no GPU is needed.

#![allow(clippy::unwrap_used, reason = "test code: an error is a test failure")]

mod support;
mod synthetic;

use std::path::PathBuf;

use blurrd_core::{
    FrameRange, JobEvent, JobId, LeakKind, LeakStatus, Project, Region, RegionSet, TrackSource,
    VerifyState,
};
use blurrd_engine::{
    AnalyzeRequest, CancellationToken, Engine, EngineConfig, OcrSetup, ProgressSink,
    ReverifyRequest,
};
use blurrd_testkit::hud::killfeed::{Kill, KillFeedSpec, Team, Weapon};
use blurrd_testkit::{Background, FontSet, Layout, Palette, Scene};
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

/// Analyses `clip` with verification.
fn analyse(engine: &Engine, clip: PathBuf, regions: RegionSet) -> Project {
    let mut req = AnalyzeRequest::new(JobId(uuid::Uuid::now_v7()), clip);
    req.regions = regions;
    req.previews = false;
    engine
        .analyze(&req, &Quiet, &CancellationToken::new())
        .unwrap()
        .project
}

/// Removes the middle `share` of the frames of every detector track that
/// `select` picks (the names are fully shown there) and marks the whole
/// video unverified, as an edit would mark its frames.
fn remove_boxes(project: &mut Project, share: f32, select: impl Fn(usize) -> bool) {
    for (index, track) in project.tracks.iter_mut().enumerate() {
        if track.source != TrackSource::Detector || !select(index) {
            continue;
        }
        let f0 = track.segments.first().unwrap().f0;
        let f1 = track.segments.last().unwrap().f1;
        let cut = ((f1 - f0 + 1) as f32 * share) as u32;
        let start = f0 + (f1 - f0 + 1 - cut) / 2;
        let end = start + cut;
        let mut kept = Vec::new();
        for segment in &track.segments {
            if segment.f0 < start {
                kept.push(blurrd_core::Segment {
                    f1: segment.f1.min(start - 1),
                    ..*segment
                });
            }
            if segment.f1 >= end {
                kept.push(blurrd_core::Segment {
                    f0: segment.f0.max(end),
                    ..*segment
                });
            }
        }
        track.segments = kept;
    }
    project.tracks.retain(|t| !t.segments.is_empty());
    project.revision += 1;
    project.verify.state = VerifyState::Unverified;
    project.verify.unverified_ranges = vec![FrameRange {
        f0: 0,
        f1: project.media.frame_count - 1,
    }];
}

#[test]
fn removed_boxes_are_found_again_and_covered() {
    let work = Workdir::new("reverify-removed");
    let Some(engine) = engine(&work) else {
        return;
    };
    let scene = synthetic::hud_scene(Layout::reference(), 150, 47);
    let clip = work.path("clip.mkv");
    synthetic::write_clip(&clip, &scene);
    let regions = RegionSet::all();
    let mut project = analyse(&engine, clip.clone(), regions);
    let analysed = synthetic::evaluate(&project, &scene, regions);

    // A third of every name's frames without boxes.
    remove_boxes(&mut project, 0.3, |_| true);
    let removed = synthetic::evaluate(&project, &scene, regions);
    assert!(removed.leaked_item_frames > 100, "{removed:?}");

    let done = engine
        .reverify_ranges(
            &ReverifyRequest::new(JobId(uuid::Uuid::now_v7()), clip, project),
            &Quiet,
            &CancellationToken::new(),
        )
        .unwrap();
    let after = synthetic::evaluate(&done.project, &scene, regions);
    let open: Vec<_> = done
        .report
        .leaks
        .iter()
        .filter(|l| l.status == LeakStatus::Open)
        .collect();
    // The removed boxes are restored: the geometric leak is back at the
    // analysis' level (frames where a name fades in or out, which OCR
    // cannot read on the source either), or every residual is reported.
    assert!(
        after.leaked_item_frames <= analysed.leaked_item_frames || !open.is_empty(),
        "analysed {analysed:?}, after {after:?}, {:?}",
        done.report.summary
    );
    assert!(done.summary.fixes > 0);
    assert_eq!(done.summary.frames, 150);
    assert_eq!(done.project.verify.state, done.report.summary.state);
    assert_eq!(done.project.verify.unverified_ranges, []);
    assert!(done.report_file.is_file());
}

#[test]
fn a_name_the_detector_missed_is_found_without_the_dictionary() {
    let work = Workdir::new("reverify-free");
    let Some(engine) = engine(&work) else {
        return;
    };
    // The killer's name is longer than any player name may be, so it never
    // enters the dictionary: only the dictionary-free class can find it.
    let long = "XxLighthouseKeeperxX";
    let kill = |killer: &str, victim: &str, birth| Kill {
        killer: killer.to_owned(),
        victim: victim.to_owned(),
        killer_team: Team::Ally,
        victim_team: Team::Enemy,
        weapon: Weapon::Rifle,
        badge: None,
        own: false,
        headshot: false,
        birth,
    };
    let scene = Scene::new(
        Layout::reference(),
        Background::noise(53),
        Palette::DEFAULT,
        120,
    )
    .with_kill_feed(
        &FontSet::latin(),
        KillFeedSpec::new(vec![
            kill(long, "Tamsin", 10),
            kill("Brannoch", "Odalys", 20),
        ]),
    );
    let clip = work.path("clip.mkv");
    synthetic::write_clip(&clip, &scene);
    let regions: RegionSet = [Region::KillFeed].into_iter().collect();
    let mut project = analyse(&engine, clip.clone(), regions);

    // The detector "missed" the long name for the middle half of its life.
    let longest = project
        .tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| t.region == Region::KillFeed)
        .max_by_key(|(_, t)| {
            let s = t.segments.first().unwrap();
            s.rect.width()
        })
        .map(|(i, _)| i)
        .unwrap();
    remove_boxes(&mut project, 0.5, |i| i == longest);
    let done = engine
        .reverify_ranges(
            &ReverifyRequest::new(JobId(uuid::Uuid::now_v7()), clip, project),
            &Quiet,
            &CancellationToken::new(),
        )
        .unwrap();
    let free: Vec<_> = done
        .report
        .leaks
        .iter()
        .filter(|l| l.kind == LeakKind::DictionaryFree)
        .collect();
    assert!(!free.is_empty(), "{:?}", done.report.leaks);
    assert!(free.iter().all(|l| l.status == LeakStatus::Fixed));
    assert_eq!(
        synthetic::evaluate(&done.project, &scene, regions).leaked_item_frames,
        0
    );
}
