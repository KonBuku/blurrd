//! Verify and fix of the screen surfaces and the full-frame sample on a
//! synthetic recording (plan 7.2 and the M17 acceptance): a scoreboard is
//! blurred and reads clean, a scoreboard whose gate missed it is found in
//! the full-frame sample and covered with slot boxes, and a name tag above
//! a head is listed as out of scope without changing the result. OCR runs
//! on the CPU, so no GPU is needed.

#![allow(clippy::unwrap_used, reason = "test code: an error is a test failure")]

mod support;
mod synthetic;

use std::path::PathBuf;

use blurrd_core::{
    FrameRange, JobEvent, JobId, LeakKind, LeakStatus, Region, RegionSet, TrackSource, VerifyState,
};
use blurrd_engine::{
    AnalyzeRequest, CancellationToken, Engine, EngineConfig, OcrSetup, ProgressSink,
    ReverifyRequest,
};
use blurrd_testkit::hud::killfeed::{Kill, KillFeedSpec, Team, Weapon};
use blurrd_testkit::hud::scoreboard::{ScoreRow, ScoreboardMode, ScoreboardSpec};
use blurrd_testkit::hud::screen::ScreenTiming;
use blurrd_testkit::hud::world_tag::WorldTag;
use blurrd_testkit::{Background, FontSet, Layout, NameStyle, Palette, Rng, Scene, synth_name};
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

/// Frames of the scene.
const FRAMES: u32 = 280;
/// When the scoreboard is open.
const BOARD: FrameRange = FrameRange { f0: 150, f1: 245 };

/// Ten players: they kill each other in the kill feed (so their names are
/// in the dictionary), then the scoreboard lists them, while a name tag
/// above a head shows one of them in the game world.
fn scene() -> Scene {
    let fonts = FontSet::latin();
    let mut rng = Rng::new(61);
    let names: Vec<String> = (0..10)
        .map(|_| synth_name(&mut rng, NameStyle::Latin(10)))
        .collect();
    let kills = (0..5)
        .map(|i| Kill {
            killer: names[2 * i].clone(),
            victim: names[2 * i + 1].clone(),
            killer_team: Team::Ally,
            victim_team: Team::Enemy,
            weapon: Weapon::Rifle,
            badge: None,
            own: false,
            headshot: false,
            birth: 10 + 20 * i as u32,
        })
        .collect();
    let rows = names
        .iter()
        .enumerate()
        .map(|(i, name)| ScoreRow {
            name: name.clone(),
            team: if i % 2 == 0 { Team::Ally } else { Team::Enemy },
            own: i == 0,
            dead: false,
        })
        .collect();
    Scene::new(
        Layout::reference(),
        Background::noise(61),
        Palette::DEFAULT,
        FRAMES,
    )
    .with_kill_feed(&fonts, KillFeedSpec::new(kills))
    .with_scoreboard(
        &fonts,
        ScoreboardSpec {
            rows,
            mode: ScoreboardMode::Teams,
            timing: ScreenTiming {
                start: BOARD.f0,
                fade_in: 0,
                hold: BOARD.f1 - BOARD.f0,
                fade_out: 0,
                slide_px: 0.0,
            },
        },
    )
    .with_world_tags(
        &fonts,
        vec![WorldTag {
            name: names[3].clone(),
            at: (1500, 760),
            shown: FrameRange {
                f0: 140,
                f1: FRAMES - 1,
            },
        }],
    )
}

#[test]
fn screen_surfaces_and_the_full_frame_sample_are_verified() {
    let work = Workdir::new("verify-surfaces");
    let Some(engine) = engine(&work) else {
        return;
    };
    let scene = scene();
    let clip = work.path("clip.mkv");
    synthetic::write_clip(&clip, &scene);
    let regions: RegionSet = [Region::KillFeed, Region::Scoreboard].into_iter().collect();
    let mut req = AnalyzeRequest::new(JobId(uuid::Uuid::now_v7()), clip.clone());
    req.regions = regions;
    req.previews = false;
    let analysis = engine
        .analyze(&req, &Quiet, &CancellationToken::new())
        .unwrap();
    let project = analysis.project;
    let report_file = analysis.report.verify_report.unwrap();
    let report: blurrd_core::VerifyReport =
        blurrd_core::schema::from_json(&std::fs::read_to_string(report_file).unwrap()).unwrap();

    // The scoreboard is boxed and reads clean.
    assert!(
        project
            .tracks
            .iter()
            .any(|t| t.region == Region::Scoreboard && t.source == TrackSource::Detector)
    );
    assert_eq!(report.summary.open, 0, "{:?}", report.leaks);
    assert_ne!(report.summary.state, VerifyState::LeaksRemain);
    assert!(report.per_region[&Region::Scoreboard].items > 0);
    let board = synthetic::evaluate(&project, &scene, regions);
    assert_eq!(
        board
            .leaks_per_region
            .get(&Region::Scoreboard)
            .copied()
            .unwrap_or(0),
        0,
        "{board:?}"
    );

    // The name tag is out of scope: listed, never boxed, never counted.
    let tags: Vec<_> = report
        .leaks
        .iter()
        .filter(|l| l.kind == LeakKind::OutOfScope)
        .collect();
    assert!(!tags.is_empty(), "{:?}", report.leaks);
    assert!(
        tags.iter()
            .all(|l| l.status == LeakStatus::Info && l.region.is_none())
    );
    assert_eq!(report.summary.out_of_scope as usize, tags.len());

    // The scoreboard's gate "missed" it: without its boxes, the full-frame
    // sample finds its names and slot boxes cover them.
    let mut missed = project.clone();
    missed.tracks.retain(|t| t.region != Region::Scoreboard);
    missed.revision += 1;
    missed.verify.state = VerifyState::Unverified;
    missed.verify.unverified_ranges = vec![FrameRange {
        f0: BOARD.f0 - 20,
        f1: BOARD.f1 + 20,
    }];
    let before = synthetic::evaluate(&missed, &scene, regions);
    assert!(
        before
            .leaks_per_region
            .get(&Region::Scoreboard)
            .copied()
            .unwrap_or(0)
            > 100,
        "{before:?}"
    );
    let done = engine
        .reverify_ranges(
            &ReverifyRequest::new(JobId(uuid::Uuid::now_v7()), clip, missed),
            &Quiet,
            &CancellationToken::new(),
        )
        .unwrap();
    let found: Vec<_> = done
        .report
        .leaks
        .iter()
        .filter(|l| l.region == Some(Region::Scoreboard))
        .collect();
    assert!(found.len() >= 3, "{:?}", done.report.leaks);
    assert!(found.iter().all(|l| l.status == LeakStatus::Fixed));
    let slots = done
        .project
        .tracks
        .iter()
        .filter(|t| t.region == Region::Scoreboard && t.source == TrackSource::OcrFix)
        .count();
    assert!(slots >= 3, "{slots} fix boxes");
    let after = synthetic::evaluate(&done.project, &scene, regions);
    let left = after
        .leaks_per_region
        .get(&Region::Scoreboard)
        .copied()
        .unwrap_or(0);
    let open = done.report.summary.open;
    assert!(
        left == 0 || open > 0,
        "{left} scoreboard item-frames left, {open} open"
    );
}
