//! The render job: blurs a video with the boxes of its project and
//! exports it.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use blurrd_blur::{CpuBlur, PixelParams};
use blurrd_core::{
    AppError, BlurParams, EncoderInfo, EncoderSetting, ErrorKind, ExportSettings, JobEvent, JobId,
    JobKind, JobSummary, MediaInfo, Project, Stage, VerifyState, WarningCode,
};
use blurrd_video::{
    BoxSource, DecodeOptions, DecodePurpose, DecoderChoice, EncodeOptions, HwDecode, MoovLayout,
    MuxOptions, PtsTable, TranscodeReport, Transcoder, VideoColour,
};
use serde::Serialize;

use crate::cache::{self, ProjectDir};
use crate::engine::Engine;
use crate::job::Job;

/// What to render.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct RenderRequest {
    /// The job's id (its events carry it).
    pub job: JobId,
    /// The video.
    pub input: PathBuf,
    /// Its project: the boxes, the blur parameters, the verification.
    pub project: Project,
    /// The output file; its extension picks the container (`.mp4`, `.mov`
    /// or `.mkv`). An existing file is replaced, the input never.
    pub output: PathBuf,
    /// Codec, encoder, quality, audio and container settings.
    pub export: ExportSettings,
    /// Name the output normally even if the project is not verified.
    pub accept_unverified: bool,
}

impl RenderRequest {
    /// Renders `input` with `project` into `output` with the default
    /// export settings.
    pub fn new(job: JobId, input: PathBuf, project: Project, output: PathBuf) -> Self {
        Self {
            job,
            input,
            project,
            output,
            export: ExportSettings::default(),
            accept_unverified: false,
        }
    }
}

/// An encoder that could not be used, and the one used instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct FallbackReport {
    /// The encoder that failed.
    pub from: String,
    /// The encoder used instead.
    pub to: String,
    /// Why it failed.
    pub reason: String,
}

/// A stream of the input that is not in the output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct SkippedReport {
    /// Its index in the input.
    pub index: u32,
    /// Its codec.
    pub codec: String,
    /// Why it was left out.
    pub reason: String,
}

/// What a render did.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[non_exhaustive]
pub struct RenderReport {
    /// The written file (named `*_UNVERIFIED` when the project is not
    /// verified).
    pub output: PathBuf,
    /// Frames written.
    pub frames: u32,
    /// Frames with at least one box.
    pub blurred_frames: u32,
    /// The encoder used.
    pub encoder: String,
    /// Encoder sessions that ran in parallel (1 or 2).
    pub sessions: u8,
    /// Wall-clock time of the job in milliseconds.
    pub elapsed_ms: u64,
    /// Frames per second over the whole job.
    pub fps: f64,
    /// Whether the project was verified.
    pub verified: bool,
    /// Whether the user accepted an unverified project (the output is
    /// then named normally although `verified` is false).
    pub accepted_unverified: bool,
    /// Encoders that failed before the one used.
    pub fallbacks: Vec<FallbackReport>,
    /// Streams of the input that are not in the output.
    pub skipped_streams: Vec<SkippedReport>,
    /// Where the MP4/MOV index is: `reserved`, `faststart`, `end`, or
    /// `none` for other containers.
    pub moov: &'static str,
}

/// What a render writes and how.
pub(crate) struct RenderTarget<'a> {
    /// The output path as requested (before `_UNVERIFIED` naming).
    pub(crate) output: &'a Path,
    /// The export settings.
    pub(crate) export: &'a ExportSettings,
    /// Whether the boxes are verified.
    pub(crate) verified: bool,
    /// Whether the user accepts unverified boxes.
    pub(crate) accept_unverified: bool,
    /// The blur parameters.
    pub(crate) blur: &'a BlurParams,
    /// The HUD scale of the video (block sizes scale with it).
    pub(crate) sy: f32,
    /// What the decoder choice optimises for.
    pub(crate) decode: DecodePurpose,
}

/// An earlier output of a render to take the unchanged frames from.
pub(crate) struct Reuse<'a> {
    /// The earlier output.
    pub(crate) previous: &'a Path,
    /// The frames whose boxes changed since it was rendered.
    pub(crate) changed: &'a [Range<u32>],
}

/// A render, ready to run: the transcoder is open (encoder chosen), the
/// blur parameters are known.
pub(crate) struct PreparedRender {
    pub(crate) transcoder: Transcoder,
    pub(crate) params: PixelParams,
    pub(crate) verified: bool,
    pub(crate) accepted_unverified: bool,
    pub(crate) total: u32,
}

impl Engine {
    /// Renders a video with the boxes of its project.
    ///
    /// The video must be the one the project was made for (same
    /// fingerprint, frames, timestamps and keyframes). The PTS table comes
    /// from the cache when the analysis left one, otherwise from one pass
    /// over the container. Outputs of projects that are not verified are
    /// named `*_UNVERIFIED` unless `req.accept_unverified` is set.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::ProjectMismatch`] when the video is not the project's;
    /// [`ErrorKind::InvalidProject`] for invalid export or blur settings or
    /// an output inside the cache; the probe, decode, encode and I/O errors
    /// of the steps; [`ErrorKind::Canceled`] when `cancel` fires (no output
    /// is left behind).
    pub fn render(
        &self,
        req: &RenderRequest,
        sink: &dyn crate::ProgressSink,
        cancel: &crate::CancellationToken,
    ) -> Result<RenderReport, AppError> {
        let job = Job::new(req.job, sink, cancel);
        let result = self.render_job(req, &job);
        job.end(result, |report| {
            finished(req.job, report, &req.project, job.warnings())
        })
    }

    fn render_job(&self, req: &RenderRequest, job: &Job<'_>) -> Result<RenderReport, AppError> {
        self.render_project(req, job, Some(JobKind::Render), None)
    }

    /// Checks and renders `req`, announcing the job as `started` (if
    /// given) once the frame count is known, with the unchanged frames
    /// taken from the earlier output of `reuse` where it can be used.
    pub(crate) fn render_project(
        &self,
        req: &RenderRequest,
        job: &Job<'_>,
        started: Option<JobKind>,
        reuse: Option<Reuse<'_>>,
    ) -> Result<RenderReport, AppError> {
        job.stage(Stage::Probe);
        let probe = self.start_encoder_probe(&req.export);
        let fingerprint = cache::fingerprint(&req.input)?;
        if !cache::same_file(&req.project.media.fingerprint, &fingerprint) {
            return Err(mismatch(String::from(
                "the file's size or content differs from the analysed video",
            )));
        }
        let info = blurrd_video::probe(&req.input)?;
        let pts = self.pts_table(&req.input, &req.project)?;
        check_media(&req.project, &info, &pts)?;
        let table = req
            .project
            .plan()
            .frame_table(&req.project.detector.regions);
        if let Some(kind) = started {
            job.started(kind, pts.len(), 1);
        }
        let target = RenderTarget {
            output: &req.output,
            export: &req.export,
            verified: is_verified(&req.project),
            accept_unverified: req.accept_unverified,
            blur: &req.project.blur,
            sy: req.project.geometry.sy,
            decode: DecodePurpose::Throughput,
        };
        let mut prepared = self.prepare_render(&target, &info, Arc::new(pts), probe, job)?;
        if let Some(reuse) = reuse {
            prepared
                .transcoder
                .reuse_previous(reuse.previous, reuse.changed);
        }
        transcode(prepared, &table, job)
    }

    /// Starts the encoder probe on its own thread when the export lets
    /// blurrd pick the encoder: it takes a second on its first run and
    /// overlaps with probing the input.
    pub(crate) fn start_encoder_probe(
        &self,
        export: &ExportSettings,
    ) -> Option<std::thread::JoinHandle<Vec<EncoderInfo>>> {
        (export.encoder == EncoderSetting::Auto && self.config().gpu).then(|| {
            let caps = self.caps_dir();
            std::thread::spawn(move || blurrd_video::probe_encoders(&caps))
        })
    }

    /// The PTS table of the project's video: the cached `pts.bin` when it
    /// is the project's, otherwise one pass over the container's packets.
    pub(crate) fn pts_table(&self, input: &Path, project: &Project) -> Result<PtsTable, AppError> {
        let dir = ProjectDir::new(&self.config().cache_dir, &project.media.fingerprint);
        if let Some(cached) = dir.read_pts()
            && cache::pts_sha256(&cached).eq_ignore_ascii_case(&project.media.pts_sha256)
        {
            return Ok(cached);
        }
        Ok(blurrd_video::pts_table(input)?)
    }

    /// Checks `target`, names the output and opens the transcoder for a
    /// video `info` with the PTS table `pts`.
    pub(crate) fn prepare_render(
        &self,
        target: &RenderTarget<'_>,
        info: &MediaInfo,
        pts: Arc<PtsTable>,
        probe: Option<std::thread::JoinHandle<Vec<EncoderInfo>>>,
        job: &Job<'_>,
    ) -> Result<PreparedRender, AppError> {
        if let Some(problem) = target.export.problem() {
            return Err(AppError::new(
                ErrorKind::InvalidProject,
                "The export settings are invalid.",
            )
            .with_detail(problem));
        }
        self.check_output(target.output)?;
        let output = if target.verified || target.accept_unverified {
            target.output.to_path_buf()
        } else {
            unverified_name(target.output)
        };
        let config = self.config();
        let mut encode = EncodeOptions::from_export(target.export);
        encode.gpu = config.gpu;
        if let Some(probe) = probe {
            encode.probed = probe.join().unwrap_or_default();
        }
        let mut decode = DecodeOptions::with_hw(if config.gpu {
            DecoderChoice::resolve(info, &self.caps_dir(), target.decode)
        } else {
            HwDecode::Software
        });
        decode.threads = config.decoder_threads;
        let colour = VideoColour::from_info(&info.color, info.height);
        let params =
            PixelParams::new(target.blur, target.sy, colour.yuv_colour()).map_err(|e| {
                AppError::new(
                    ErrorKind::InvalidProject,
                    "The project's blur settings are invalid.",
                )
                .with_detail(e.to_string())
            })?;
        let total = pts.len();
        let transcoder = Transcoder::open(
            info,
            pts,
            &output,
            &decode,
            &encode,
            MuxOptions::from_export(target.export),
        )?;
        for fallback in transcoder.fallbacks() {
            job.warning(
                WarningCode::EncoderFallback,
                format!(
                    "{} could not be used ({}); encoding with {}",
                    fallback.from, fallback.reason, fallback.to
                ),
                None,
            );
        }
        Ok(PreparedRender {
            transcoder,
            params,
            verified: target.verified,
            accepted_unverified: !target.verified && target.accept_unverified,
            total,
        })
    }

    /// Refuses outputs inside the cache folder (plan 10).
    fn check_output(&self, output: &Path) -> Result<(), AppError> {
        let resolve = |path: &Path| {
            let parent = path.parent().filter(|p| !p.as_os_str().is_empty());
            parent
                .and_then(|p| p.canonicalize().ok())
                .unwrap_or_else(|| parent.unwrap_or(Path::new(".")).to_path_buf())
        };
        let cache = self
            .config()
            .cache_dir
            .canonicalize()
            .unwrap_or_else(|_| self.config().cache_dir.clone());
        if resolve(output).starts_with(&cache) {
            return Err(AppError::new(
                ErrorKind::InvalidProject,
                "The output cannot be written into blurrd's cache folder.",
            )
            .with_detail(output.display().to_string()));
        }
        Ok(())
    }
}

/// Runs a prepared render with the boxes of `boxes`.
pub(crate) fn transcode(
    prepared: PreparedRender,
    boxes: &dyn BoxSource,
    job: &Job<'_>,
) -> Result<RenderReport, AppError> {
    let PreparedRender {
        transcoder,
        params,
        verified,
        accepted_unverified,
        total,
    } = prepared;
    job.stage(Stage::BlurEncode);
    let stop = || job.is_canceled();
    let report = transcoder.run(boxes, &CpuBlur, &params, &stop, |n| job.progress(n, total))?;
    if report.reused_frames > 0 {
        tracing::info!(
            reused = report.reused_frames,
            encoded = report.frames - report.reused_frames,
            "took the unchanged frames from the earlier output"
        );
    }
    for skipped in &report.skipped_streams {
        tracing::warn!(stream = skipped.index, codec = %skipped.codec,
            reason = %skipped.reason, "stream not in the output");
    }
    Ok(render_report(&report, verified, accepted_unverified, job))
}

/// The report of a finished transcode.
fn render_report(
    report: &TranscodeReport,
    verified: bool,
    accepted_unverified: bool,
    job: &Job<'_>,
) -> RenderReport {
    let elapsed = job.elapsed();
    RenderReport {
        output: report.output.clone(),
        frames: report.frames,
        blurred_frames: report.blurred_frames,
        encoder: report.encoder.clone(),
        sessions: report.sessions,
        elapsed_ms: u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
        fps: f64::from(report.frames) / elapsed.as_secs_f64().max(1e-9),
        verified,
        accepted_unverified,
        fallbacks: report
            .fallbacks
            .iter()
            .map(|f| FallbackReport {
                from: f.from.clone(),
                to: f.to.clone(),
                reason: f.reason.clone(),
            })
            .collect(),
        skipped_streams: report
            .skipped_streams
            .iter()
            .map(|s| SkippedReport {
                index: s.index,
                codec: s.codec.clone(),
                reason: s.reason.clone(),
            })
            .collect(),
        moov: match report.moov {
            MoovLayout::Reserved { .. } => "reserved",
            MoovLayout::Faststart => "faststart",
            MoovLayout::AtEnd => "end",
            _ => "none",
        },
    }
}

/// The `Finished` event of a render of `project`.
pub(crate) fn finished(
    job: JobId,
    report: &RenderReport,
    project: &Project,
    warnings: u32,
) -> JobEvent {
    JobEvent::Finished {
        job,
        summary: JobSummary {
            frames: report.frames,
            elapsed_ms: report.elapsed_ms,
            output: Some(report.output.clone()),
            // The project's own state; a clean verification of an older
            // revision (or with unverified ranges) no longer counts.
            verify: Some(match project.verify.state {
                VerifyState::Clean if !report.verified => VerifyState::Unverified,
                state => state,
            }),
            warnings,
        },
    }
}

/// A project mismatch with `detail`.
fn mismatch(detail: String) -> AppError {
    AppError::new(
        ErrorKind::ProjectMismatch,
        "The video is not the one the project was made for.",
    )
    .with_detail(detail)
}

/// Checks that the video is the one the project describes: picture size,
/// frame count, timestamps and keyframes.
pub(crate) fn check_media(
    project: &Project,
    info: &MediaInfo,
    pts: &PtsTable,
) -> Result<(), AppError> {
    let media = &project.media;
    if (media.width, media.height) != (info.width, info.height) {
        return Err(mismatch(format!(
            "the project is for {}x{}, the video is {}x{}",
            media.width, media.height, info.width, info.height
        )));
    }
    if media.frame_count != pts.len() {
        return Err(mismatch(format!(
            "the project has {} frames, the video {}",
            media.frame_count,
            pts.len()
        )));
    }
    if !media
        .pts_sha256
        .eq_ignore_ascii_case(&cache::pts_sha256(pts))
    {
        return Err(mismatch(String::from(
            "the frame timestamps differ from the project's",
        )));
    }
    let keyframes = cache::keyframes_sha256(info.keyframes.as_deref());
    if !media.keyframes_sha256.eq_ignore_ascii_case(&keyframes) {
        return Err(mismatch(String::from(
            "the keyframes differ from the project's",
        )));
    }
    Ok(())
}

/// What the render adds to the file stem of an output whose project is
/// not verified (see [`unverified_name`]).
pub const UNVERIFIED_SUFFIX: &str = "_UNVERIFIED";

/// Whether the project's last verification is clean and current: the
/// render names its output [`unverified_name`] otherwise (unless the
/// request accepts unverified output).
pub fn is_verified(project: &Project) -> bool {
    project.verify.state == VerifyState::Clean
        && project.verify.plan_revision == project.revision
        && project.verify.unverified_ranges.is_empty()
}

/// `name_UNVERIFIED.ext` next to `name.ext`: the name of an output whose
/// project is not verified.
pub fn unverified_name(output: &Path) -> PathBuf {
    let stem = output.file_stem().unwrap_or_default().to_string_lossy();
    let name = match output.extension() {
        Some(ext) => format!("{stem}{UNVERIFIED_SUFFIX}.{}", ext.to_string_lossy()),
        None => format!("{stem}{UNVERIFIED_SUFFIX}"),
    };
    output.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::unverified_name;

    #[test]
    fn unverified_outputs_are_marked_in_the_name() {
        assert_eq!(
            unverified_name(Path::new("D:/out/clip_blurrd.mp4")),
            Path::new("D:/out/clip_blurrd_UNVERIFIED.mp4")
        );
        assert_eq!(
            unverified_name(Path::new("clip")),
            Path::new("clip_UNVERIFIED")
        );
    }
}
