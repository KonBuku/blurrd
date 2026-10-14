//! The analysis job on real recordings from `BLURRD_TEST_MEDIA` (local
//! only, never in CI): throughput without OCR, the temporal models'
//! look-ahead, and the plan 11.3 gates without OCR on every reviewed
//! golden excerpt, scored on the boxes of the analysed project. Only
//! counts are printed: no names, no text.
//!
//! ```text
//! cargo nextest run -p blurrd-engine --release --run-ignored all real_analysis --no-capture
//! ```
//!
//! `BLURRD_ANALYSIS_CLIPS=<n>` limits the throughput run to the first `n`
//! clips of the manifest (default: all), `BLURRD_ANALYSIS_GPU=1` lets the
//! decoder choice use the GPU (default: software decoding, the choice
//! without OCR on H.264 8-bit sources, plan 8).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "local measurement: a missing file or a failed decode is a test failure"
)]

use std::collections::BTreeSet;
use std::path::Path;

use blurrd_blur::{PixelParams, PreparedBox, YuvColour, prepare_boxes};
use blurrd_core::{BlurParams, JobEvent, JobId, Project, Region, RegionSet};
use blurrd_engine::{
    Analysis, AnalyzeRequest, CancellationToken, Engine, EngineConfig, LOOKAHEAD_MAX, ProgressSink,
};
use blurrd_testkit::golden::{self, Golden};
use blurrd_testkit::media::{Manifest, MediaDir};

/// Ignores events.
struct Quiet;

impl ProgressSink for Quiet {
    fn emit(&self, _: JobEvent) {}
}

fn engine() -> (Engine, std::path::PathBuf) {
    let cache = std::env::temp_dir().join(format!("blurrd-real-analysis-{}", std::process::id()));
    let mut config = EngineConfig::new(cache.clone());
    config.gpu = std::env::var("BLURRD_ANALYSIS_GPU").is_ok_and(|v| v == "1");
    (Engine::new(config).unwrap(), cache)
}

fn analyze(engine: &Engine, clip: &Path) -> Analysis {
    let mut req = AnalyzeRequest::new(JobId(uuid::Uuid::now_v7()), clip.to_path_buf());
    req.verify = false;
    engine
        .analyze(&req, &Quiet, &CancellationToken::new())
        .unwrap()
}

#[test]
#[ignore = "needs BLURRD_TEST_MEDIA"]
fn real_analysis_throughput_and_lookahead() {
    let Some(media) = MediaDir::from_env() else {
        eprintln!("skipped: BLURRD_TEST_MEDIA is not set");
        return;
    };
    let manifest = Manifest::load_repository().unwrap();
    let limit = std::env::var("BLURRD_ANALYSIS_CLIPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(usize::MAX);
    let (engine, cache) = engine();
    let (mut frames, mut seconds) = (0u64, 0f64);
    for clip in manifest.clips.iter().take(limit) {
        let analysis = analyze(&engine, &media.clip_path(clip));
        let report = &analysis.report;
        assert_eq!(report.frames, clip.frames, "{}", clip.id);
        assert!(report.max_delay <= LOOKAHEAD_MAX, "{}", clip.id);
        let per_region = |region: Region| {
            analysis
                .project
                .tracks
                .iter()
                .filter(|t| t.region == region)
                .count()
        };
        eprintln!(
            "{}: {} frames, {:.0} fps ({}), max delay {}, tracks {} (feed {}, banner {}, voice {}, \
             chat {}, screens {})",
            clip.id,
            report.frames,
            report.fps,
            report.decoder,
            report.max_delay,
            report.tracks,
            per_region(Region::KillFeed),
            per_region(Region::Banner),
            per_region(Region::VoiceChat),
            per_region(Region::TextChat),
            Region::SCREENS
                .iter()
                .map(|&r| per_region(r))
                .sum::<usize>(),
        );
        frames += u64::from(report.frames);
        seconds += report.elapsed.as_secs_f64();
    }
    eprintln!(
        "total: {frames} frames in {seconds:.1} s, {:.0} fps",
        frames as f64 / seconds.max(1e-9)
    );
    let _ = std::fs::remove_dir_all(cache);
}

/// The prepared boxes of `project` on every frame, for `regions`.
fn prepared(project: &Project, regions: &RegionSet) -> impl FnMut(u32) -> Vec<PreparedBox> {
    let params = PixelParams::new(
        &BlurParams::default(),
        project.geometry.sy,
        YuvColour::default(),
    )
    .unwrap();
    let table = project.plan().frame_table(regions);
    let (width, height) = (project.media.width, project.media.height);
    move |frame| {
        let mut out = Vec::new();
        prepare_boxes(table.boxes(frame), width, height, &params, &mut out);
        out
    }
}

#[test]
#[ignore = "needs BLURRD_TEST_MEDIA"]
fn real_analysis_passes_the_golden_gates() {
    let Some(media) = MediaDir::from_env() else {
        eprintln!("skipped: BLURRD_TEST_MEDIA is not set");
        return;
    };
    let manifest = Manifest::load_repository().unwrap();
    let (engine, cache) = engine();
    let mut evaluated = 0;
    for excerpt in &manifest.excerpts {
        let path = media.golden_path(excerpt);
        if !path.is_file() {
            eprintln!("{}: no golden (not verified)", excerpt.id);
            continue;
        }
        let golden = Golden::from_json(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let clip = manifest.clip(&excerpt.clip).unwrap();
        let analysis = analyze(&engine, &media.clip_path(clip));
        // Scored on the regions the golden reviews; boxes of other regions
        // would count as false positives against it.
        let regions: RegionSet = golden
            .items
            .iter()
            .map(|item| item.region)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let summary = golden::evaluate(&golden, prepared(&analysis.project, &regions));
        let fps =
            f64::from(analysis.project.media.fps.num) / f64::from(analysis.project.media.fps.den);
        eprintln!(
            "{} ({}): {} item-frames, leaked {}, over-blur {:.3}, portrait {}, false positives \
             {:.2} box-s, jitter {}, flicker {}",
            excerpt.id,
            if manifest.is_held_out(excerpt) {
                "held out"
            } else {
                "tuning"
            },
            summary.item_frames,
            summary.leaked_item_frames,
            summary.over_blur.ratio(),
            summary.portrait_pixels,
            summary.false_positive_seconds(fps),
            summary.jitter_frames,
            summary.flicker_runs,
        );
        assert_eq!(summary.leaked_item_frames, 0, "{}: {summary:?}", excerpt.id);
        assert!(
            summary.over_blur.ratio() <= 0.35,
            "{}: {summary:?}",
            excerpt.id
        );
        assert_eq!(summary.portrait_pixels, 0, "{}: {summary:?}", excerpt.id);
        assert_eq!(summary.flicker_runs, 0, "{}: {summary:?}", excerpt.id);
        evaluated += 1;
    }
    eprintln!("{evaluated} goldens evaluated");
    let _ = std::fs::remove_dir_all(cache);
}
