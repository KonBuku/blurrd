//! The analysis job: decode, detect, track; writes the project.

mod calibrate;
pub(crate) mod frame;
pub(crate) mod pass0;
mod pipeline;
mod previews;
mod stages;
pub(crate) mod tracks;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use blurrd_blur::PixelParams;
use blurrd_core::{
    AppError, BlurParams, Coverage, DetectorInfo, ErrorKind, HudGeometry, HudLayout, JobEvent,
    JobId, JobKind, JobSummary, MediaInfo, Mode, Project, ProjectMedia, ProjectVerify, RegionSet,
    Sensitivity, Stage, TemporalParams, VerifyReport, VerifyState, VerifySummary, WarningCode,
};
use blurrd_detect::killfeed::BarHues;
use blurrd_video::{
    DecodeOptions, DecodePurpose, DecoderChoice, EncodeOptions, HwDecode, PtsTable, RgbConverter,
    VideoColour,
};
use serde::Serialize;

use self::frame::Cropper;
use self::pass0::{Pass0, Warming};
use self::pipeline::{Pipeline, PipelineOutput};
use self::previews::Previews;
use self::tracks::TrackBuilder;
use crate::cache::{self, ProjectDir};
use crate::dictionary::Pass0Stats;
use crate::engine::Engine;
use crate::hud::HudMap;
use crate::job::{Job, internal};
use crate::verify::{self, Context, PixelSetup, Pixels, StructureLog};

/// The most frames any temporal model may hold a frame before its boxes
/// are final (plan 5.4): banner episodes need about 180, the kill-feed
/// ring 128. The streaming `run` renders each frame at most this many
/// frames behind the analysis.
pub const LOOKAHEAD_MAX: u32 = 240;

/// What to analyse.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct AnalyzeRequest {
    /// The job's id (its events carry it).
    pub job: JobId,
    /// The video.
    pub input: PathBuf,
    /// The regions to detect.
    pub regions: RegionSet,
    /// Blur parameters stored in the project.
    pub blur: BlurParams,
    /// Whether to verify: reads the protected zones with OCR (pass 0),
    /// checks the blurred result and fixes what stays readable when OCR
    /// is available, and warns when it is not.
    pub verify: bool,
    /// Which frames the verification reads.
    pub coverage: Coverage,
    /// Whether to make the thumbnails and, for videos the desktop app's
    /// player cannot play, the preview proxy.
    pub previews: bool,
}

impl AnalyzeRequest {
    /// Analyses every supported region of `input` with the default blur,
    /// with verification and previews.
    pub fn new(job: JobId, input: PathBuf) -> Self {
        Self {
            job,
            input,
            regions: RegionSet::all(),
            blur: BlurParams::default(),
            verify: true,
            coverage: Coverage::Structure,
            previews: true,
        }
    }
}

/// What an analysis did.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[non_exhaustive]
pub struct AnalyzeReport {
    /// The project file in the cache.
    pub project_file: PathBuf,
    /// Frames analysed.
    pub frames: u32,
    /// Tracks found.
    pub tracks: usize,
    /// The decoder used (`software`, `nvdec` or `d3d11va`).
    pub decoder: &'static str,
    /// Wall-clock time in milliseconds.
    pub elapsed_ms: u64,
    /// Frames per second over the whole job.
    pub fps: f64,
    /// The largest look-ahead of a temporal model in frames (at most
    /// [`LOOKAHEAD_MAX`]).
    pub max_delay: u32,
    /// Pass 0 counters, when OCR ran.
    pub pass0: Option<Pass0Stats>,
    /// The verification's result, when it ran.
    pub verify: Option<VerifySummary>,
    /// The verification report in the cache, when it ran.
    pub verify_report: Option<PathBuf>,
    /// Time the verification took after the detectors (pass 0 runs during
    /// them), in milliseconds, when it ran.
    pub verify_ms: Option<u64>,
    /// Thumbnails written.
    pub thumbnails: u32,
    /// The preview proxy, when the video needed one.
    pub proxy: Option<PathBuf>,
}

/// A finished analysis: the project (also saved in the cache) and the
/// report.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Analysis {
    /// The project.
    pub project: Project,
    /// What the analysis did.
    pub report: AnalyzeReport,
}

/// The name of a decoder back end in reports.
pub(crate) fn decoder_name(hw: HwDecode) -> &'static str {
    match hw {
        HwDecode::Nvdec => "nvdec",
        HwDecode::D3d11va => "d3d11va",
        _ => "software",
    }
}

/// Everything the analysis of one video needs before its pipeline starts.
pub(crate) struct Prepared {
    pub(crate) info: MediaInfo,
    pub(crate) fingerprint: blurrd_core::Fingerprint,
    pub(crate) geometry: HudGeometry,
    pub(crate) map: HudMap,
    /// Tight, or Safe where the calibration cannot be trusted.
    pub(crate) mode: Mode,
    /// The kill feed's learned bar colours.
    pub(crate) hues: Option<BarHues>,
    pub(crate) colour: VideoColour,
    pub(crate) converter: RgbConverter,
    pub(crate) decode: DecodeOptions,
    pub(crate) pass0: Option<Pass0>,
}

/// What a verification after the detectors produced.
struct Verified {
    report: VerifyReport,
    path: PathBuf,
    /// Its time in milliseconds.
    ms: u64,
}

impl Engine {
    /// Probes the video of `req` and prepares its analysis: calibrated
    /// geometry, colour conversion, decoder, OCR. Emits the warnings that
    /// apply to the whole video. `beside_render` tells that a render
    /// decodes the same video at the same time (a streaming run).
    pub(crate) fn prepare_analysis(
        &self,
        req: &AnalyzeRequest,
        beside_render: bool,
        job: &Job<'_>,
    ) -> Result<Prepared, AppError> {
        let input = req.input.as_path();
        job.stage(Stage::Probe);
        let fingerprint = cache::fingerprint(input)?;
        let header = blurrd_video::probe_header(input)?;
        // The blur scales its blocks with the HUD (with the frame height in
        // every layout); refuse videos whose HUD it cannot scale to before
        // spending an analysis on them.
        let colour = VideoColour::from_info(&header.color, header.height);
        let sy = HudGeometry::for_layout(header.width, header.height, HudLayout::Auto).sy;
        if let Err(error) = PixelParams::new(&BlurParams::default(), sy, colour.yuv_colour()) {
            return Err(AppError::new(
                ErrorKind::UnsupportedMedia,
                "The video is too small or too large to blur reliably.",
            )
            .with_detail(format!("{}x{}: {error}", header.width, header.height)));
        }
        // The OCR engines are built while the video is probed and
        // calibrated.
        let warming = self.warm_up_ocr(req, (header.width, header.height));
        // The calibration reads its samples while the probe scans the
        // packets; both come before the analysis can start.
        let (info, samples) = std::thread::scope(|scope| {
            let scan = scope.spawn(|| blurrd_video::probe(input));
            let samples = calibrate::read_samples(input, &header, colour, job);
            match scan.join() {
                Ok(info) => (info, samples),
                Err(panic) => std::panic::resume_unwind(panic),
            }
        });
        let info = info?;
        let calibration = calibrate::calibrate(input, &info, colour, samples?, job)?;
        let geometry = calibration.geometry;
        let mut mode = Mode::Tight;
        if let Some(doubt) = calibration.doubt {
            mode = Mode::Safe;
            job.warning(
                WarningCode::CalibrationLow,
                format!(
                    "In this {}x{} video {doubt}. Safe boxes cover the whole name slots; check \
                     the result.",
                    info.width, info.height
                ),
                None,
            );
        }
        let converter = match calibration.paper_white {
            Some(nits) => RgbConverter::with_paper_white(colour, nits),
            None => RgbConverter::new(colour),
        };
        if converter.is_tone_mapped() && calibration.paper_white.is_none() {
            mode = Mode::Safe;
            job.warning(
                WarningCode::HdrToneMapped,
                String::from(
                    "The video is HDR and the brightness of its HUD could not be measured; the \
                     detectors read it tone-mapped with a reference white, and Safe boxes cover \
                     the whole name slots.",
                ),
                None,
            );
        }
        let pass0 = if req.verify {
            self.open_pass0(&geometry, &info, req, warming, job)
        } else {
            None
        };
        let config = self.config();
        let hw = if config.gpu {
            // The detectors, not the decoder, bound an analysis, so it
            // leaves the CPU to them and OCR where the GPU decodes fast
            // enough. Beside a render without OCR the analysis takes the
            // fastest decoder and the render the other one.
            let purpose = if pass0.is_some() || !beside_render {
                DecodePurpose::SpareCpu
            } else {
                DecodePurpose::Throughput
            };
            DecoderChoice::resolve(&info, &self.caps_dir(), purpose)
        } else {
            HwDecode::Software
        };
        let mut decode = DecodeOptions::with_hw(hw);
        decode.threads = config.decoder_threads;
        Ok(Prepared {
            map: HudMap::new(&geometry, info.width, info.height),
            info,
            fingerprint,
            geometry,
            mode,
            hues: calibration.hues,
            colour,
            converter,
            decode,
            pass0,
        })
    }

    /// Starts building the OCR engines of a verifying analysis of a
    /// `size` video, when OCR is installed.
    fn warm_up_ocr(&self, req: &AnalyzeRequest, size: (u32, u32)) -> Option<Warming> {
        let config = self.config();
        let setup = config.ocr.as_ref().filter(|_| req.verify)?;
        match Pass0::warm_up(setup, config.adapter, size, req.regions, Pass0::MAX_READERS) {
            Ok(warming) => Some(warming),
            Err(reason) => {
                tracing::debug!(%reason, "the OCR engines are built after the calibration");
                None
            }
        }
    }

    /// Opens pass 0 with the engines `warming` built, or warns that the
    /// result stays unverified.
    fn open_pass0(
        &self,
        geometry: &HudGeometry,
        info: &MediaInfo,
        req: &AnalyzeRequest,
        warming: Option<Warming>,
        job: &Job<'_>,
    ) -> Option<Pass0> {
        let config = self.config();
        let warmed = match warming.map(std::thread::JoinHandle::join) {
            Some(Ok(Ok(warmed))) => Some(warmed),
            Some(Ok(Err(reason))) => {
                job.warning(
                    WarningCode::OcrUnavailable,
                    format!("OCR cannot run ({reason}); the result is not verified."),
                    None,
                );
                return None;
            }
            Some(Err(_)) | None => None,
        };
        let reason = match &config.ocr {
            Some(setup) => match Pass0::open(
                setup,
                config.adapter,
                geometry,
                info,
                req.regions,
                req.coverage,
                Pass0::MAX_READERS,
                warmed,
            ) {
                Ok(pass0) => return Some(pass0),
                Err(reason) => reason,
            },
            None => String::from("ONNX Runtime and the OCR models are not installed"),
        };
        job.warning(
            WarningCode::OcrUnavailable,
            format!("OCR cannot run ({reason}); the result is not verified."),
            None,
        );
        None
    }

    /// Analyses a video: detects and tracks the names of `req.regions`,
    /// builds the OCR name dictionary when verifying, and saves the
    /// project (with the PTS table) in the cache. A previous project of
    /// the same video keeps its overrides.
    ///
    /// # Errors
    ///
    /// The probe, decode and I/O errors of the steps; [`ErrorKind::Canceled`]
    /// when `cancel` fires.
    pub fn analyze(
        &self,
        req: &AnalyzeRequest,
        sink: &dyn crate::ProgressSink,
        cancel: &crate::CancellationToken,
    ) -> Result<Analysis, AppError> {
        let job = Job::new(req.job, sink, cancel);
        let result = self.analyze_job(req, &job);
        job.end(result, |analysis| JobEvent::Finished {
            job: req.job,
            summary: JobSummary {
                frames: analysis.report.frames,
                elapsed_ms: analysis.report.elapsed_ms,
                output: Some(analysis.report.project_file.clone()),
                verify: Some(analysis.project.verify.state),
                warnings: job.warnings(),
            },
        })
    }

    fn analyze_job(&self, req: &AnalyzeRequest, job: &Job<'_>) -> Result<Analysis, AppError> {
        let prepared = self.prepare_analysis(req, false, job)?;
        let expected = expected_frames(&prepared.info);
        job.started(JobKind::Analyze, expected);
        self.run_analysis(req, prepared, job, Some(expected), &mut |_, _| {})
    }

    /// Runs the prepared analysis of `req` and saves the project and the
    /// PTS table in the cache. `progress` and `on_frame` as for
    /// [`Pipeline`].
    pub(crate) fn run_analysis(
        &self,
        req: &AnalyzeRequest,
        prepared: Prepared,
        job: &Job<'_>,
        progress: Option<u32>,
        on_frame: &mut dyn FnMut(u32, &TrackBuilder),
    ) -> Result<Analysis, AppError> {
        let Prepared {
            info,
            fingerprint,
            geometry,
            map,
            mode,
            hues,
            colour,
            converter,
            decode,
            pass0,
        } = prepared;
        let decoder = decoder_name(decode.hw);
        let dir = ProjectDir::new(&self.config().cache_dir, &fingerprint);
        dir.create()?;
        let previews = req.previews.then(|| {
            let mut proxy = EncodeOptions::default();
            proxy.encoder = self.config().proxy_encoder.clone();
            proxy.gpu = self.config().gpu;
            Previews::open(&dir, &info, converter.clone(), &proxy)
        });
        job.stage(Stage::Detect);
        let pixels = PixelSetup::source(
            decode.clone(),
            &req.blur,
            &geometry,
            colour,
            converter.clone(),
            req.regions,
            (info.width, info.height),
        )?;
        let reads = pass0.as_ref().map(Pass0::reads);
        let pipeline = Pipeline {
            input: &req.input,
            decode,
            cropper: Cropper::new(map, colour, converter.clone(), req.regions, reads),
            stages: stages::stages(req.regions, map, mode, hues.as_ref()),
            sources: stages::sources(req.regions),
            pass0,
            previews,
            pools: self.config().pools,
            progress,
        };
        let output = pipeline.run(job, on_frame)?;
        check_output(&output, job)?;
        let previews = output.previews.unwrap_or_default();
        let pts = Arc::new(PtsTable::new(info.time_base, output.pts)?);
        pts.write(&dir.pts())?;
        let media = project_media(req, &info, fingerprint, &pts);
        let detector = DetectorInfo {
            version: format!("detect/{}", blurrd_core::VERSION),
            regions: req.regions,
            mode,
            sensitivity: Sensitivity::default(),
            temporal: TemporalParams::default(),
        };
        let mut project = Project::new(media, geometry, detector, req.blur);
        project.tracks = output.tracks;
        if let Some(previous) = dir.read_project() {
            project.overrides = previous.overrides;
            project.revision = previous.revision + 1;
        }
        project.verify.state = VerifyState::Unverified;
        let pass0_stats = output.pass0.as_ref().map(Pass0::stats);
        let verified = match output.pass0 {
            Some(mut pass0) => {
                let source = VerifySource {
                    input: &req.input,
                    pts: Arc::clone(&pts),
                    pixels,
                    dir: &dir,
                };
                let verified = verify_or_warn(&mut project, &mut pass0, source, job);
                self.release(pass0);
                verified?
            }
            None => None,
        };
        cache::write_project(&project, &dir.project())?;
        let elapsed = job.elapsed();
        let frames = pts.len();
        Ok(Analysis {
            report: AnalyzeReport {
                project_file: dir.project(),
                frames,
                tracks: project.tracks.len(),
                decoder,
                elapsed_ms: u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
                fps: f64::from(frames) / elapsed.as_secs_f64().max(1e-9),
                max_delay: output.max_delay,
                pass0: pass0_stats,
                verify: verified.as_ref().map(|v| v.report.summary),
                verify_ms: verified.as_ref().map(|v| v.ms),
                verify_report: verified.map(|v| v.path),
                thumbnails: previews.thumbnails,
                proxy: previews.proxy,
            },
            project,
        })
    }
}

/// Checks what the pipeline produced and warns about what stopped on the
/// way (OCR, previews).
fn check_output(output: &PipelineOutput, job: &Job<'_>) -> Result<(), AppError> {
    if let Some(reason) = &output.pass0_error {
        job.warning(
            WarningCode::OcrUnavailable,
            format!("OCR stopped ({reason}); the result is not verified."),
            None,
        );
    }
    if output.max_delay > LOOKAHEAD_MAX {
        return Err(internal(format!(
            "a temporal model held frames {} frames, more than {LOOKAHEAD_MAX}",
            output.max_delay
        )));
    }
    if output.pts.is_empty() {
        return Err(no_frames());
    }
    for problem in output.previews.iter().flat_map(|p| &p.problems) {
        job.warning(WarningCode::PreviewUnavailable, problem.clone(), None);
    }
    Ok(())
}

/// Where the verification after the detectors reads blurred pixels and
/// writes its report.
struct VerifySource<'a> {
    /// The video.
    input: &'a Path,
    /// Its PTS table.
    pts: Arc<PtsTable>,
    /// How its blurred pixels are made.
    pixels: PixelSetup,
    /// Its cache folder.
    dir: &'a ProjectDir,
}

/// [`verify_project`], or a warning and no result when the verification
/// fails for another reason than a cancellation.
fn verify_or_warn(
    project: &mut Project,
    pass0: &mut Pass0,
    source: VerifySource<'_>,
    job: &Job<'_>,
) -> Result<Option<Verified>, AppError> {
    job.stage(Stage::Verify);
    let started = std::time::Instant::now();
    match verify_project(project, pass0, source, job) {
        Ok(verified) => Ok(Some(Verified {
            ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            ..verified
        })),
        Err(error) if error.kind == ErrorKind::Canceled => Err(error),
        Err(error) => {
            job.warning(
                WarningCode::OcrUnavailable,
                format!("The verification stopped ({error}); the result is not verified."),
                None,
            );
            Ok(None)
        }
    }
}

/// Verifies `project` with what `pass0` read and adds the fixes it made,
/// reading blurred pixels from `source`. Writes the report next to the
/// project and records it in the project.
fn verify_project(
    project: &mut Project,
    pass0: &mut Pass0,
    source: VerifySource<'_>,
    job: &Job<'_>,
) -> Result<Verified, AppError> {
    let VerifySource {
        input,
        pts,
        pixels,
        dir,
    } = source;
    let coverage = pass0.coverage();
    let parts = pass0.parts();
    // The audit and the verification of edited ranges read the source
    // again without the detectors; they confirm name slots with these.
    StructureLog::from_lines(parts.log, project.media.frame_count).write(&dir.structure())?;
    let ctx = Context {
        log: parts.log,
        whitelist: parts.whitelist,
        geometry: &project.geometry,
        size: (project.media.width, project.media.height),
        regions: project.detector.regions,
    };
    let mut reader = Pixels::open(input, pts, pixels, parts.ocr)?;
    let outcome = verify::verify(&ctx, &project.plan(), &mut reader, None, job)?;
    drop(reader);
    verify::warn(&outcome, job);
    project.tracks.extend(outcome.fixes.iter().cloned());
    let report = verify::report(
        &outcome,
        project.revision,
        project.media.fingerprint.clone(),
        coverage,
    );
    let name = verify::report_name(project.revision);
    let path = dir.project().with_file_name(&name);
    verify::write_report(&report, &path)?;
    project.verify = ProjectVerify {
        plan_revision: project.revision,
        state: report.summary.state,
        unverified_ranges: Vec::new(),
        report: Some(name),
    };
    Ok(Verified {
        report,
        path,
        ms: 0,
    })
}

/// The media section of the project of `req`'s video, described by
/// `info`, with its fingerprint and PTS table.
fn project_media(
    req: &AnalyzeRequest,
    info: &MediaInfo,
    fingerprint: blurrd_core::Fingerprint,
    pts: &PtsTable,
) -> ProjectMedia {
    ProjectMedia {
        path: req.input.clone(),
        fingerprint,
        width: info.width,
        height: info.height,
        fps: info.fps,
        time_base: info.time_base,
        frame_count: pts.len(),
        pts_sha256: cache::pts_sha256(pts),
        is_vfr: info.is_vfr,
        has_b_frames: info.has_b_frames,
        keyframes_sha256: cache::keyframes_sha256(info.keyframes.as_deref()),
    }
}

/// The number of frames `info` promises: the container's count, or the
/// duration times the frame rate when the container has none (Matroska).
/// Only for progress; the decoded frames are what counts.
pub(crate) fn expected_frames(info: &MediaInfo) -> u32 {
    info.frame_count.unwrap_or_else(|| {
        let fps = f64::from(info.fps.num) / f64::from(info.fps.den);
        (info.duration_s * fps)
            .round()
            .clamp(0.0, f64::from(u32::MAX)) as u32
    })
}

/// An analysis error for a video without frames.
pub(crate) fn no_frames() -> AppError {
    AppError::new(ErrorKind::UnsupportedMedia, "The video has no frames.")
}
