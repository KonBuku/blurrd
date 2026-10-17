//! Verifying the frame ranges an edit touched again (plan 7.2 step 7).
//!
//! An override (a box the user added, moved or removed) changes what the
//! output shows on its frames, so the verification of the analysis no
//! longer holds there: the project lists those frames as unverified
//! ranges. This job decodes only those ranges (seeking to each), reads
//! every frame's protected zones with OCR as pass 0 does (the dictionary
//! lives only in memory, so it is read again), checks the blurred result
//! against the project's current plan and adds fix boxes where a name
//! stays readable. The report then covers the whole video: the leaks of
//! the previous report outside the ranges, the new ones inside.

use std::path::PathBuf;
use std::sync::Arc;

use blurrd_core::{
    AppError, Coverage, ErrorKind, FrameRange, JobEvent, JobId, JobKind, JobSummary, Project,
    ProjectVerify, RegionSet, Stage, VerifyReport, schema,
};
use blurrd_video::{
    DecodeOptions, DecodePurpose, Decoder, DecoderChoice, HwDecode, RgbConverter, VideoColour,
};
use serde::Serialize;

use crate::analysis::frame::Cropper;
use crate::analysis::pass0::Pass0;
use crate::cache::{self, ProjectDir};
use crate::engine::Engine;
use crate::hud::HudMap;
use crate::job::{Job, canceled};
use crate::render::check_media;
use crate::verify::{self, Context, PixelSetup, Pixels};

/// What to verify again.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ReverifyRequest {
    /// The job's id (its events carry it).
    pub job: JobId,
    /// The video.
    pub input: PathBuf,
    /// Its project, with the unverified ranges to check.
    pub project: Project,
}

impl ReverifyRequest {
    /// Verifies the unverified ranges of `project`, the project of
    /// `input`.
    pub fn new(job: JobId, input: PathBuf, project: Project) -> Self {
        Self {
            job,
            input,
            project,
        }
    }
}

/// What a verification of ranges produced.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Reverify {
    /// The project with the fix boxes added and its verification updated
    /// (also saved in the cache).
    pub project: Project,
    /// The report of the whole video.
    pub report: VerifyReport,
    /// The report file in the cache.
    pub report_file: PathBuf,
    /// What the job did.
    pub summary: ReverifySummary,
}

/// The counts of a verification of ranges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct ReverifySummary {
    /// The ranges checked.
    pub ranges: u32,
    /// Frames decoded.
    pub frames: u32,
    /// Fix boxes added.
    pub fixes: u32,
    /// Wall-clock time in milliseconds.
    pub elapsed_ms: u64,
}

impl Engine {
    /// Verifies again the frame ranges of a project that edits touched
    /// (`project.verify.unverified_ranges`; the whole video when the
    /// project was never verified, or has no current verification and
    /// lists no range), adds fix
    /// boxes where a name stays readable, and saves the project and the
    /// report in the cache.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::ProjectMismatch`] when the video is not the project's;
    /// [`ErrorKind::OcrUnavailable`] without OCR; decoding, OCR and I/O
    /// errors; [`ErrorKind::Canceled`] when `cancel` fires (the cached
    /// project is left as it was).
    pub fn reverify_ranges(
        &self,
        req: &ReverifyRequest,
        sink: &dyn crate::ProgressSink,
        cancel: &crate::CancellationToken,
    ) -> Result<Reverify, AppError> {
        let job = Job::new(req.job, sink, cancel);
        let result = self.reverify_job(req, &job);
        job.end(result, |done| JobEvent::Finished {
            job: req.job,
            summary: JobSummary {
                frames: done.summary.frames,
                elapsed_ms: done.summary.elapsed_ms,
                output: Some(done.report_file.clone()),
                verify: Some(done.project.verify.state),
                warnings: job.warnings(),
            },
        })
    }

    fn reverify_job(&self, req: &ReverifyRequest, job: &Job<'_>) -> Result<Reverify, AppError> {
        job.stage(Stage::Probe);
        let project = req.project.clone();
        let fingerprint = cache::fingerprint(&req.input)?;
        if !cache::same_file(&project.media.fingerprint, &fingerprint) {
            return Err(AppError::new(
                ErrorKind::ProjectMismatch,
                "The video is not the one the project was made for.",
            )
            .with_detail("the file's size or content differs from the analysed video"));
        }
        let info = blurrd_video::probe(&req.input)?;
        let pts = Arc::new(self.pts_table(&req.input, &project)?);
        check_media(&project, &info, &pts)?;
        let ranges = ranges_to_check(&project);
        let total: u32 = ranges.iter().map(|r| r.f1 - r.f0 + 1).sum();
        job.started(JobKind::ReverifyRanges, total);
        let config = self.config();
        let setup = config.ocr.as_ref().ok_or_else(|| {
            AppError::new(
                ErrorKind::OcrUnavailable,
                "OCR is not installed, so the edited frames cannot be verified.",
            )
        })?;
        let regions = project.detector.regions;
        let mut pass0 = Pass0::open(
            setup,
            config.adapter,
            &project.geometry,
            &info,
            regions,
            Coverage::EveryFrame,
        )
        .map_err(|reason| {
            AppError::new(ErrorKind::OcrUnavailable, "OCR cannot run.").with_detail(reason)
        })?;
        let colour = VideoColour::from_info(&info.color, info.height);
        // The analysis's paper white of an HDR video is not kept; the
        // reference white reads the same HUD text.
        let converter = RgbConverter::new(colour);
        let mut decode = DecodeOptions::with_hw(if config.gpu {
            DecoderChoice::resolve(&info, &self.caps_dir(), DecodePurpose::SpareCpu)
        } else {
            HwDecode::Software
        });
        decode.threads = config.decoder_threads;

        job.stage(Stage::Ocr);
        let map = HudMap::new(&project.geometry, info.width, info.height);
        let mut cropper =
            Cropper::new(map, colour, converter.clone(), regions, Some(pass0.reads()));
        let source = Source {
            input: &req.input,
            decode: &decode,
            pts: &pts,
            ranges: &ranges,
            total,
        };
        let frames = read_ranges(&source, &mut pass0, &mut cropper, job)?;

        job.stage(Stage::Verify);
        let pixels = PixelSetup::source(
            decode,
            &project.blur,
            &project.geometry,
            colour,
            converter,
            regions,
            (info.width, info.height),
        )?;
        let parts = pass0.parts();
        let ctx = Context {
            log: parts.log,
            whitelist: parts.whitelist,
            geometry: &project.geometry,
            size: (info.width, info.height),
            regions,
        };
        let mut reader = Pixels::open(&req.input, pts, pixels, parts.ocr)?;
        let outcome = verify::verify(&ctx, &project.plan(), &mut reader, Some(&ranges), job)?;
        drop(reader);
        verify::warn(&outcome, job);
        let mut done = self.save(project, &outcome, &ranges)?;
        done.summary.frames = frames;
        done.summary.elapsed_ms = u64::try_from(job.elapsed().as_millis()).unwrap_or(u64::MAX);
        Ok(done)
    }

    /// Adds the fixes of `outcome` (a verification of `ranges`) to
    /// `project`, merges its report with the project's previous one and
    /// saves both in the cache.
    fn save(
        &self,
        mut project: Project,
        outcome: &verify::Outcome,
        ranges: &[FrameRange],
    ) -> Result<Reverify, AppError> {
        let dir = ProjectDir::new(&self.config().cache_dir, &project.media.fingerprint);
        dir.create()?;
        let previous = project
            .verify
            .report
            .as_ref()
            .and_then(|name| std::fs::read_to_string(dir.project().with_file_name(name)).ok())
            .and_then(|text| schema::from_json::<VerifyReport>(&text).ok());
        let fixes = u32::try_from(outcome.fixes.len()).unwrap_or(u32::MAX);
        if fixes > 0 {
            project.tracks.extend(outcome.fixes.iter().cloned());
            project.revision += 1;
        }
        let report = merged_report(&project, outcome, previous, ranges);
        let name = verify::report_name(project.revision);
        let report_file = dir.project().with_file_name(&name);
        verify::write_report(&report, &report_file)?;
        project.verify = ProjectVerify {
            plan_revision: project.revision,
            state: report.summary.state,
            unverified_ranges: Vec::new(),
            report: Some(name),
        };
        cache::write_project(&project, &dir.project())?;
        Ok(Reverify {
            project,
            report,
            report_file,
            summary: ReverifySummary {
                ranges: u32::try_from(ranges.len()).unwrap_or(u32::MAX),
                frames: 0,
                fixes,
                elapsed_ms: 0,
            },
        })
    }
}

/// The ranges of a video to read.
struct Source<'a> {
    input: &'a std::path::Path,
    decode: &'a DecodeOptions,
    pts: &'a blurrd_video::PtsTable,
    ranges: &'a [FrameRange],
    /// Frames in all ranges.
    total: u32,
}

/// Reads every frame of the ranges of `source` with `pass0` as pass 0
/// reads a video (the crops cut by `cropper`); returns the frames read.
fn read_ranges(
    source: &Source<'_>,
    pass0: &mut Pass0,
    cropper: &mut Cropper,
    job: &Job<'_>,
) -> Result<u32, AppError> {
    let mut decoder = Decoder::open(source.input, source.decode)?;
    let mut frames = 0u32;
    let stopped = || job.is_canceled();
    for range in source.ranges {
        decoder.seek(range.f0, source.pts)?;
        for _ in range.f0..=range.f1 {
            let Some(frame) = decoder.next_frame()? else {
                break;
            };
            let crops = cropper.crops(frame)?;
            // Without the detectors every region counts as showing its
            // structure; the plan's boxes still confirm the slots.
            let read = pass0
                .read(&crops, true, RegionSet::all(), &stopped)
                .map_err(|reason| {
                    AppError::new(ErrorKind::OcrUnavailable, "OCR stopped.").with_detail(reason)
                })?;
            if !read {
                return Err(canceled());
            }
            frames += 1;
            job.progress(frames, source.total);
        }
    }
    Ok(frames)
}

/// The ranges to check: the project's unverified ranges, merged where
/// they overlap or touch; the whole video when the project was never
/// verified (no report covers the frames outside the ranges) or lists no
/// range and has no current verification.
fn ranges_to_check(project: &Project) -> Vec<FrameRange> {
    let verified = project.verify.report.is_some();
    let current = verified && project.verify.plan_revision == project.revision;
    let whole = !verified || (project.verify.unverified_ranges.is_empty() && !current);
    let mut ranges = if whole {
        project
            .media
            .frame_count
            .checked_sub(1)
            .map(|f1| vec![FrameRange { f0: 0, f1 }])
            .unwrap_or_default()
    } else {
        project.verify.unverified_ranges.clone()
    };
    ranges.sort_by_key(|r| r.f0);
    let mut merged: Vec<FrameRange> = Vec::with_capacity(ranges.len());
    for range in ranges {
        match merged.last_mut() {
            Some(last) if range.f0 <= last.f1.saturating_add(1) => last.f1 = last.f1.max(range.f1),
            _ => merged.push(range),
        }
    }
    merged
}

/// The report of the whole video after a verification of `ranges`: the
/// leaks of `previous` outside the ranges and the new ones; the counts
/// per region of `previous` (the detector tracks did not change; OCR read
/// the same source), or the new ones without a previous report.
fn merged_report(
    project: &Project,
    outcome: &verify::Outcome,
    previous: Option<VerifyReport>,
    ranges: &[FrameRange],
) -> VerifyReport {
    let touches = |r: &FrameRange| ranges.iter().any(|x| r.f0 <= x.f1 && x.f0 <= r.f1);
    let (mut leaks, per_region, frames_checked) = match previous {
        Some(previous) => (
            previous
                .leaks
                .into_iter()
                .filter(|l| !touches(&l.frame_range))
                .collect(),
            previous.per_region,
            previous.frames_checked.max(outcome.frames_checked),
        ),
        None => (
            Vec::new(),
            outcome.per_region.clone(),
            outcome.frames_checked,
        ),
    };
    leaks.extend(outcome.leaks.iter().cloned());
    leaks.sort_by_key(|l| (l.frame_range.f0, l.frame_range.f1));
    VerifyReport::new(
        project.revision,
        project.media.fingerprint.clone(),
        Coverage::Structure,
        frames_checked,
        per_region,
        leaks,
    )
}

#[cfg(test)]
mod tests {
    use blurrd_core::{
        BlurParams, DetectorInfo, Fingerprint, FrameRange, HudGeometry, Mode, Project,
        ProjectMedia, Rational, RegionSet, Sensitivity, TemporalParams,
    };

    use super::ranges_to_check;

    fn project(frames: u32) -> Project {
        let media = ProjectMedia {
            path: "clip.mp4".into(),
            fingerprint: Fingerprint {
                size: 1,
                mtime: 0,
                blake3_head_tail: "00".repeat(32),
            },
            width: 1920,
            height: 1080,
            fps: Rational::new(60, 1).unwrap(),
            time_base: Rational::new(1, 60).unwrap(),
            frame_count: frames,
            pts_sha256: "00".repeat(32),
            is_vfr: false,
            has_b_frames: false,
            keyframes_sha256: "00".repeat(32),
        };
        let detector = DetectorInfo {
            version: String::from("detect/test"),
            regions: RegionSet::all(),
            mode: Mode::Tight,
            sensitivity: Sensitivity::default(),
            temporal: TemporalParams::default(),
        };
        Project::new(
            media,
            HudGeometry::default(),
            detector,
            BlurParams::default(),
        )
    }

    #[test]
    fn ranges_merge_and_default_to_the_whole_video() {
        let mut project = project(600);
        project.verify.report = Some(String::from("verify-0.json"));
        project.revision = 1;
        project.verify.unverified_ranges = vec![
            FrameRange::new(300, 340).unwrap(),
            FrameRange::new(10, 20).unwrap(),
            FrameRange::new(341, 360).unwrap(),
            FrameRange::new(15, 30).unwrap(),
        ];
        assert_eq!(
            ranges_to_check(&project),
            [
                FrameRange::new(10, 30).unwrap(),
                FrameRange::new(300, 360).unwrap()
            ]
        );
        // Verified at an older revision, no range listed: everything.
        project.verify.unverified_ranges.clear();
        assert_eq!(
            ranges_to_check(&project),
            [FrameRange::new(0, 599).unwrap()]
        );
        // Verified and current: nothing.
        project.verify.plan_revision = 1;
        assert_eq!(ranges_to_check(&project), []);
    }

    #[test]
    fn a_never_verified_project_is_checked_whole_despite_its_ranges() {
        let mut project = project(600);
        project.verify.unverified_ranges = vec![FrameRange::new(10, 20).unwrap()];
        assert_eq!(
            ranges_to_check(&project),
            [FrameRange::new(0, 599).unwrap()]
        );
    }
}
