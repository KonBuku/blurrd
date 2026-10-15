//! The work of each job kind: turns the frontend's request into an
//! engine request and runs it with the job's sink.
//!
//! The engine reports `Started` through `Finished`, `Failed` or
//! `Canceled` itself; a request that cannot reach the engine (a file
//! that was never analysed, an output name that is not valid) fails with
//! the same events.

use std::sync::Arc;
use std::time::Instant;

use blurrd_core::{
    AppError, BlurParams, ExportSettings, JobEvent, JobKind, JobSummary, Region, RegionSet,
    VerifyState, WarningCode,
};
use blurrd_engine::{AnalyzeRequest, Engine, ProgressSink, RenderRequest, RunRequest};

use crate::jobs::{JobSink, Work};
use crate::output::{OutputTarget, output_target};
use crate::projects::{OpenProject, is_verified};

/// What an export needs besides the project.
#[derive(Debug, Clone)]
pub(crate) struct ExportRequest {
    /// Codec, encoder, container and output naming.
    pub(crate) export: ExportSettings,
    /// The blur.
    pub(crate) blur: BlurParams,
    /// The detector regions that are blurred (manual boxes always are).
    pub(crate) enabled: RegionSet,
}

/// The regions of `regions` as a set.
pub(crate) fn region_set(regions: &[Region]) -> RegionSet {
    regions.iter().copied().collect()
}

/// Analyses `project`: detects `regions` and verifies when `verify` is set.
pub(crate) fn analysis(
    engine: Arc<Engine>,
    project: Arc<OpenProject>,
    regions: RegionSet,
    verify: bool,
) -> Work {
    Box::new(move |sink| {
        let mut req = AnalyzeRequest::new(sink.job(), project.source().to_path_buf());
        req.regions = regions;
        req.verify = verify;
        if let Some(existing) = project.project() {
            req.blur = existing.blur;
        }
        let result = engine.analyze(&req, sink, sink.cancel());
        project.forget_pts();
        log_failure("analysis", result.err());
    })
}

/// Renders `project` with `request`.
pub(crate) fn render(
    engine: Arc<Engine>,
    project: Arc<OpenProject>,
    request: ExportRequest,
) -> Work {
    Box::new(move |sink| {
        let prepared = render_request(&project, &request, sink);
        match prepared {
            Ok(Some(req)) => match engine.render(&req, sink, sink.cancel()) {
                Ok(report) => project.set_last_output(report.output),
                Err(error) => log_failure("render", Some(error)),
            },
            Ok(None) => {}
            Err(error) => sink.failed(error),
        }
    })
}

/// Exports `project` as one file of a run: renders it when its analysis
/// covers the requested regions and no edit waits for verification,
/// otherwise analyses and renders it in one streaming pass.
pub(crate) fn run(
    engine: Arc<Engine>,
    project: Arc<OpenProject>,
    request: ExportRequest,
    regions: RegionSet,
    verify: bool,
) -> Work {
    Box::new(move |sink| {
        let current = project.project().is_some_and(|existing| {
            existing.verify.unverified_ranges.is_empty()
                && regions
                    .iter()
                    .all(|region| existing.detector.regions.contains(region))
        });
        if current {
            return render(engine, project, request)(sink);
        }
        let output = match output_target(project.source(), &request.export, false) {
            Ok(OutputTarget::Write(path)) => path,
            Ok(OutputTarget::Skip(existing)) => return skipped(sink, &existing),
            Err(error) => return sink.failed(error),
        };
        let mut req = RunRequest::new(sink.job(), project.source().to_path_buf(), output);
        req.regions = regions;
        req.blur = request.blur;
        req.export = request.export;
        req.verify = verify;
        let result = engine.run(&req, sink, sink.cancel());
        project.forget_pts();
        match result {
            Ok(report) => project.set_last_output(report.render.output),
            Err(error) => log_failure("run", Some(error)),
        }
    })
}

/// Re-verifies the frame ranges edits changed. The engine cannot verify
/// single ranges yet, so the ranges stay unverified: the job says so
/// with a warning and finishes `Unverified`, and the export that follows
/// is named `_UNVERIFIED`.
pub(crate) fn reverify(project: Arc<OpenProject>) -> Work {
    Box::new(move |sink| {
        let started = Instant::now();
        let ranges = match project.analysed() {
            Ok(existing) => existing.verify.unverified_ranges,
            Err(error) => return sink.failed(error),
        };
        let frames = ranges
            .iter()
            .map(|range| range.f1 - range.f0 + 1)
            .sum::<u32>();
        let job = sink.job();
        sink.emit(JobEvent::Started {
            job,
            kind: JobKind::ReverifyRanges,
            total_frames: frames,
        });
        sink.emit(JobEvent::Warning {
            job,
            code: WarningCode::UnverifiableItems,
            message: "The edited frames could not be verified again; the export will be marked \
                      unverified."
                .to_owned(),
            frames: ranges.first().zip(ranges.last()).map(|(a, b)| (a.f0, b.f1)),
        });
        sink.emit(JobEvent::Finished {
            job,
            summary: JobSummary {
                frames,
                elapsed_ms: started.elapsed().as_millis() as u64,
                output: None,
                verify: Some(VerifyState::Unverified),
                warnings: 1,
            },
        });
    })
}

/// The engine request of a render, or `None` when the output exists and
/// the conflict policy skips it (the job is finished then).
fn render_request(
    project: &OpenProject,
    request: &ExportRequest,
    sink: &JobSink,
) -> Result<Option<RenderRequest>, AppError> {
    let mut analysed = project.analysed()?;
    analysed.blur = request.blur;
    analysed.detector.regions = request.enabled;
    let verified = is_verified(&analysed);
    let output = match output_target(project.source(), &request.export, verified)? {
        OutputTarget::Write(path) => path,
        OutputTarget::Skip(existing) => {
            skipped(sink, &existing);
            return Ok(None);
        }
    };
    let mut req = RenderRequest::new(sink.job(), project.source().to_path_buf(), analysed, output);
    req.export = request.export.clone();
    Ok(Some(req))
}

/// Finishes a job whose output exists and is kept (conflict policy Skip).
fn skipped(sink: &JobSink, existing: &std::path::Path) {
    tauri_plugin_log::log::info!(
        "export skipped, the output exists: {}",
        existing.file_name().unwrap_or_default().to_string_lossy()
    );
    sink.emit(JobEvent::Finished {
        job: sink.job(),
        summary: JobSummary {
            frames: 0,
            elapsed_ms: 0,
            output: None,
            verify: None,
            warnings: 0,
        },
    });
}

/// Logs a job's error (its events already told the frontend).
fn log_failure(job: &str, error: Option<AppError>) {
    if let Some(error) = error {
        tauri_plugin_log::log::info!("{job} ended: {error}");
    }
}

/// The jobs on a real recording (`BLURRD_TEST_CLIP`, or the first MP4 in
/// `BLURRD_TEST_MEDIA`): CPU only (software decoding, FFV1 output, no
/// OCR). Run in release builds; they print what they measure.
#[cfg(test)]
pub(crate) mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use blurrd_core::{
        BlurParams, Container, EncoderSetting, ExportSettings, JobEvent, JobId, Override, RectI,
        Region, RegionSet,
    };
    use blurrd_engine::{Engine, EngineConfig};
    use uuid::Uuid;

    use super::{ExportRequest, analysis, render};
    use crate::jobs::tests::{CLOCK_SLACK, Recorder};
    use crate::jobs::{Jobs, PROGRESS_INTERVAL};
    use crate::paths::tests::paths_below;
    use crate::projects::{OpenProject, Projects};

    /// The test recording, if one is configured.
    pub(crate) fn test_clip() -> Option<PathBuf> {
        if let Some(clip) = std::env::var_os("BLURRD_TEST_CLIP") {
            return Some(PathBuf::from(clip));
        }
        let media = PathBuf::from(std::env::var_os("BLURRD_TEST_MEDIA")?);
        let mut clips: Vec<PathBuf> = std::fs::read_dir(media)
            .ok()?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "mp4"))
            .collect();
        clips.sort();
        clips.into_iter().next()
    }

    /// A CPU-only engine and an opened project of `clip` in a fresh cache.
    pub(crate) fn open(
        clip: &std::path::Path,
        name: &str,
    ) -> (Arc<Engine>, Arc<OpenProject>, PathBuf) {
        let work = std::env::temp_dir().join(format!("blurrd-app-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&work);
        std::fs::create_dir_all(&work).unwrap();
        let mut config = EngineConfig::new(work.join("cache"));
        config.gpu = false;
        let engine = Arc::new(Engine::new(config).unwrap());
        let paths = paths_below(clip.parent().unwrap());
        let source = paths.existing(clip).unwrap();
        let project = Projects::new(work.join("cache")).open(&source).unwrap();
        (engine, project, work)
    }

    /// Checks the progress of `job`: frames never go back, events are at
    /// least 50 ms apart (but the last); answers their count and rate.
    fn check_progress(recorder: &Recorder, job: JobId) -> (usize, f64) {
        let events = recorder.events.lock().unwrap();
        let progress: Vec<(Instant, u32, u32)> = events
            .iter()
            .filter_map(|(at, event)| match *event {
                JobEvent::Progress {
                    job: j,
                    frame,
                    total,
                    ..
                } if j == job => Some((*at, frame, total)),
                _ => None,
            })
            .collect();
        for pair in progress.windows(2) {
            assert!(pair[1].1 >= pair[0].1, "progress went backwards");
            let gap = pair[1].0.duration_since(pair[0].0);
            assert!(
                gap + CLOCK_SLACK >= PROGRESS_INTERVAL || pair[1].1 == pair[1].2,
                "{gap:?}"
            );
        }
        let span = progress
            .first()
            .zip(progress.last())
            .map_or(Duration::ZERO, |(a, b)| b.0.duration_since(a.0));
        let rate = (progress.len().saturating_sub(1)) as f64 / span.as_secs_f64().max(1e-9);
        (progress.len(), rate)
    }

    #[test]
    #[ignore = "needs BLURRD_TEST_MEDIA; run in release"]
    fn jobs_report_ordered_throttled_progress_and_edits_round_trip() {
        let Some(clip) = test_clip() else {
            eprintln!("skipped: BLURRD_TEST_MEDIA is not set");
            return;
        };
        let (engine, project, work) = open(&clip, "jobs");
        let jobs = Jobs::start().unwrap();
        let recorder = Arc::new(Recorder::default());

        let job = JobId(Uuid::now_v7());
        let started = Instant::now();
        jobs.enqueue(
            job,
            recorder.clone(),
            analysis(
                Arc::clone(&engine),
                Arc::clone(&project),
                RegionSet::all(),
                false,
            ),
        );
        recorder.wait_end(job);
        let (count, rate) = check_progress(&recorder, job);
        eprintln!(
            "analysis: {:.1} s, {count} progress events, {rate:.1} per second",
            started.elapsed().as_secs_f64()
        );
        assert!(rate <= 20.5, "{rate}");
        assert!(matches!(
            recorder.events.lock().unwrap().last(),
            Some((_, JobEvent::Finished { .. }))
        ));

        // The project reads back from the cache; an edit saves at the
        // current revision only.
        let tracks = project.tracks();
        assert_ne!(tracks.tracks, vec![]);
        let edit = Override::Add {
            uid: Uuid::now_v7(),
            region: Region::Manual,
            f0: 10,
            f1: 20,
            rect: RectI::new(100, 100, 300, 140).unwrap(),
        };
        let saved = project.set_overrides(tracks.revision, vec![edit]).unwrap();
        assert_eq!(saved.revision, tracks.revision + 1);
        assert!(project.set_overrides(tracks.revision, Vec::new()).is_err());
        assert_eq!(project.tracks().overrides, vec![edit]);
        assert_eq!(
            project.frame_pts().unwrap().pts.len(),
            tracks_frames(&project)
        );

        // A render to FFV1, cancelled after half a second.
        let mut export = ExportSettings {
            encoder: EncoderSetting::Named("ffv1".to_owned()),
            container: Container::Mkv,
            ..ExportSettings::default()
        };
        export.output.folder = blurrd_core::OutputFolder::Folder(work.clone());
        let request = ExportRequest {
            export,
            blur: BlurParams::default(),
            enabled: RegionSet::all(),
        };
        let job = JobId(Uuid::now_v7());
        jobs.enqueue(
            job,
            recorder.clone(),
            render(engine, Arc::clone(&project), request),
        );
        std::thread::sleep(Duration::from_millis(500));
        let asked = Instant::now();
        jobs.cancel(job);
        recorder.wait_end(job);
        let latency = asked.elapsed();
        eprintln!("render cancel: {} ms", latency.as_millis());
        assert!(matches!(
            recorder.events.lock().unwrap().last(),
            Some((_, JobEvent::Canceled { .. }))
        ));
        let left: Vec<_> = std::fs::read_dir(&work)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_file())
            .collect();
        assert!(left.is_empty(), "a cancelled render left {left:?}");
        jobs.shutdown();
        std::fs::remove_dir_all(&work).unwrap();
    }

    /// Run jobs through the app's queue against the engine's own run on
    /// the same recording, with GPU decoding and encoding and without OCR
    /// (as the CLI throughput target is measured): the queue, the sink and
    /// the channel filter must keep at least 95 % of the throughput.
    #[test]
    #[ignore = "needs BLURRD_TEST_MEDIA and a GPU; run in release under the GPU lock"]
    fn run_jobs_keep_the_engines_throughput() {
        use blurrd_engine::{CancellationToken, ProgressSink, RunRequest};

        struct Quiet;
        impl ProgressSink for Quiet {
            fn emit(&self, _: JobEvent) {}
        }

        let Some(clip) = test_clip() else {
            eprintln!("skipped: BLURRD_TEST_MEDIA is not set");
            return;
        };
        let work = std::env::temp_dir().join(format!("blurrd-app-speed-{}", std::process::id()));
        let cache = work.join("cache");
        let engine = Arc::new(Engine::new(EngineConfig::new(cache.clone())).unwrap());
        let paths = paths_below(clip.parent().unwrap());
        let source = paths.existing(&clip).unwrap();
        let jobs = Jobs::start().unwrap();
        // Every run analyses anew; the capability caches (decoder
        // benchmark, encoder probe) stay, as they do for the app and the CLI.
        let fresh = || {
            let _ = std::fs::remove_dir_all(cache.join("projects"));
            for entry in std::fs::read_dir(&work).into_iter().flatten().flatten() {
                if entry.path().is_file() {
                    std::fs::remove_file(entry.path()).unwrap();
                }
            }
            std::fs::create_dir_all(&work).unwrap();
        };
        let mut direct = Vec::new();
        let mut queued = Vec::new();
        for round in 0..6 {
            fresh();
            let started = Instant::now();
            let req = RunRequest::new(
                JobId(Uuid::now_v7()),
                source.clone(),
                work.join("direct.mp4"),
            );
            engine.run(&req, &Quiet, &CancellationToken::new()).unwrap();
            let engine_time = started.elapsed();

            fresh();
            let project = Projects::new(cache.clone()).open(&source).unwrap();
            let mut export = ExportSettings::default();
            export.output.folder = blurrd_core::OutputFolder::Folder(work.clone());
            let request = ExportRequest {
                export,
                blur: BlurParams::default(),
                enabled: RegionSet::all(),
            };
            let recorder = Arc::new(Recorder::default());
            let job = JobId(Uuid::now_v7());
            let started = Instant::now();
            jobs.enqueue(
                job,
                recorder.clone(),
                super::run(
                    Arc::clone(&engine),
                    project,
                    request,
                    RegionSet::all(),
                    true,
                ),
            );
            recorder.wait_end(job);
            let app_time = started.elapsed();
            assert!(matches!(
                recorder.events.lock().unwrap().last(),
                Some((_, JobEvent::Finished { .. }))
            ));
            eprintln!(
                "round {round}: engine run {:.2} s, app run job {:.2} s",
                engine_time.as_secs_f64(),
                app_time.as_secs_f64()
            );
            // The first round warms the decoder and encoder caches.
            if round > 0 {
                direct.push(engine_time.as_secs_f64());
                queued.push(app_time.as_secs_f64());
            }
        }
        let best = |times: &[f64]| times.iter().copied().fold(f64::INFINITY, f64::min);
        let ratio = best(&direct) / best(&queued);
        eprintln!("app throughput / engine throughput = {ratio:.3}");
        assert!(ratio >= 0.95, "{ratio}");
        jobs.shutdown();
        let _ = std::fs::remove_dir_all(&work);
    }

    fn tracks_frames(project: &OpenProject) -> usize {
        project.analysed().unwrap().media.frame_count as usize
    }
}
