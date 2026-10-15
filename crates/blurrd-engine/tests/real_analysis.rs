//! The analysis job on real recordings from `BLURRD_TEST_MEDIA` (local
//! only, never in CI): throughput without OCR, the temporal models'
//! look-ahead, and the plan 11.3 gates without OCR on every reviewed
//! golden excerpt, scored on the boxes of the analysed project. Only
//! counts are printed: no names, no text.
//!
//! False positives are split in two. A box that touches no name on its
//! frame but the same place's name within [`ROLL`] frames is a deliberate
//! pre-roll or post-roll (the trackers hold boxes a few frames before a
//! name fades in and after it fades out); it is reported, not gated.
//! Every other false positive counts against the plan 11.3 gate of 1
//! box-second per 10 minutes of held-out footage. The golden excerpts are
//! short and dense (a few seconds around names), so the gate cannot be
//! measured as a rate on them: no excerpt may have any (tuning and
//! held-out alike).
//!
//! ```text
//! cargo nextest run -p blurrd-engine --release --run-ignored all real_analysis --no-capture
//! ```
//!
//! `BLURRD_ANALYSIS_CLIPS=<n>` limits the throughput run to the first `n`
//! clips of the manifest (default: all), `BLURRD_ANALYSIS_GPU=1` lets the
//! decoder choice use the GPU as the product does (NVDEC on NVIDIA GPUs;
//! default: software decoding, so the test needs no GPU).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "local measurement: a missing file or a failed decode is a test failure"
)]

use std::collections::BTreeSet;
use std::path::Path;

use blurrd_blur::{PixelParams, PreparedBox, YuvColour, prepare_boxes};
use blurrd_core::{BlurParams, JobEvent, JobId, Project, RectI, Region, RegionSet};
use blurrd_engine::{
    Analysis, AnalyzeRequest, CancellationToken, Engine, EngineConfig, LOOKAHEAD_MAX, ProgressSink,
};
use blurrd_testkit::golden::{self, Golden};
use blurrd_testkit::media::{Manifest, MediaDir};

/// The largest number of frames a tracker holds a box before or after its
/// name (pre-rolls and post-rolls of every region, with margin).
const ROLL: u32 = 12;

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
    req.previews = false;
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
        seconds += report.elapsed_ms as f64 / 1000.0;
    }
    eprintln!(
        "total: {frames} frames in {seconds:.1} s, {:.0} fps",
        frames as f64 / seconds.max(1e-9)
    );
    let _ = std::fs::remove_dir_all(cache);
}

/// The false-positive box-frames of `boxes` against `golden`: those
/// within [`ROLL`] frames of a name they cover then, and the others.
fn false_positives(golden: &Golden, mut boxes: impl FnMut(u32) -> Vec<PreparedBox>) -> (u64, u64) {
    let (mut rolls, mut others) = (0, 0);
    let near = |frame: u32, core: &RectI| {
        golden.items.iter().any(|item| {
            (frame.saturating_sub(ROLL)..=frame + ROLL)
                .filter_map(|f| item.rect_at(f))
                .any(|r| r.intersection(core).is_some())
        })
    };
    for frame in golden.frames.f0..=golden.frames.f1 {
        let truth = golden.truth_at(frame);
        let touched: Vec<RectI> = truth
            .items
            .iter()
            .map(|i| i.glyphs.bounds().unwrap_or(i.text))
            .collect();
        for b in boxes(frame) {
            let core = b.core();
            if touched.iter().any(|t| core.intersection(t).is_some()) {
                continue;
            }
            if near(frame, &core) {
                rolls += 1;
            } else {
                others += 1;
            }
        }
    }
    (rolls, others)
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
        let (rolls, others) = false_positives(&golden, prepared(&analysis.project, &regions));
        assert_eq!(rolls + others, summary.false_positive_box_frames);
        let fps =
            f64::from(analysis.project.media.fps.num) / f64::from(analysis.project.media.fps.den);
        eprintln!(
            "{} ({}): {} item-frames, leaked {}, over-blur {:.3}, portrait {}, false positives \
             {:.2} box-s (rolls {:.2}, other {:.2}), jitter {}, flicker {}",
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
            rolls as f64 / fps,
            others as f64 / fps,
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
        assert_eq!(others, 0, "{}: false positives outside rolls", excerpt.id);
        evaluated += 1;
    }
    eprintln!("{evaluated} goldens evaluated");
    let _ = std::fs::remove_dir_all(cache);
}
