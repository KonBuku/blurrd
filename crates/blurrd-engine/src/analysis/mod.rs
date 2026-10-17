//! The analysis job: decode, detect, track; writes the project.

mod calibrate;
mod frame;
mod pass0;
mod pipeline;
mod previews;
mod stages;
pub(crate) mod tracks;

use std::path::PathBuf;

use blurrd_blur::PixelParams;
use blurrd_core::{
    AppError, BlurParams, DetectorInfo, ErrorKind, HudGeometry, HudLayout, JobEvent, JobId,
    JobKind, JobSummary, MediaInfo, Mode, Project, ProjectMedia, RegionSet, Sensitivity, Stage,
    TemporalParams, VerifyState, WarningCode,
};
use blurrd_detect::killfeed::BarHues;
use blurrd_video::{
    DecodeOptions, DecodePurpose, DecoderChoice, EncodeOptions, HwDecode, PtsTable, RgbConverter,
    VideoColour,
};
use serde::Serialize;

use self::frame::Cropper;
use self::pass0::Pass0;
use self::pipeline::Pipeline;
use self::previews::Previews;
use self::tracks::TrackBuilder;
use crate::cache::{self, ProjectDir};
use crate::dictionary::Pass0Stats;
use crate::engine::Engine;
use crate::hud::HudMap;
use crate::job::{Job, internal};

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
    /// Whether to verify: builds the OCR name dictionary (pass 0) when OCR
    /// is available, and warns when it is not.
    pub verify: bool,
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

impl Engine {
    /// Probes `input` and prepares its analysis: calibrated geometry,
    /// colour conversion, decoder, OCR. Emits the warnings that apply to
    /// the whole video. `beside_render` tells that a render decodes the same
    /// video at the same time (a streaming run).
    pub(crate) fn prepare_analysis(
        &self,
        input: &std::path::Path,
        verify: bool,
        beside_render: bool,
        job: &Job<'_>,
    ) -> Result<Prepared, AppError> {
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
        let pass0 = if verify {
            self.open_pass0(&geometry, &info, job)
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

    /// Opens pass 0, or warns that the result stays unverified.
    fn open_pass0(&self, geometry: &HudGeometry, info: &MediaInfo, job: &Job<'_>) -> Option<Pass0> {
        let config = self.config();
        let reason = match &config.ocr {
            Some(setup) => {
                match Pass0::open(setup, config.adapter, geometry, info.width, info.height) {
                    Ok(pass0) => return Some(pass0),
                    Err(reason) => reason,
                }
            }
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
        let prepared = self.prepare_analysis(&req.input, req.verify, false, job)?;
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
        let pipeline = Pipeline {
            input: &req.input,
            decode,
            cropper: Cropper::new(map, colour, converter.clone(), req.regions, pass0.is_some()),
            stages: stages::stages(req.regions, map, mode, hues.as_ref()),
            sources: stages::sources(req.regions),
            pass0,
            previews,
            pools: self.config().pools,
            progress,
        };
        let output = pipeline.run(job, on_frame)?;
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
        let previews = output.previews.unwrap_or_default();
        for problem in &previews.problems {
            job.warning(WarningCode::PreviewUnavailable, problem.clone(), None);
        }
        let pts = PtsTable::new(info.time_base, output.pts)?;
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
                pass0: output.pass0,
                thumbnails: previews.thumbnails,
                proxy: previews.proxy,
            },
            project,
        })
    }
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
