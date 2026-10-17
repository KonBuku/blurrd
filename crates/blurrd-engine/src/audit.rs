//! The audit of a rendered output (plan 7.2 step 8): OCR over the decoded
//! output as an end-to-end check of what the encoder wrote.
//!
//! The verification during the analysis reads blurred crops made in
//! memory exactly as the render makes them; the audit reads the file the
//! render wrote, with the encoder's loss. It reads the protected zones of
//! the source that pass 0 of the analysis read, on the same frames (the
//! dictionary lives only in memory; the cache keeps which zones were read,
//! [`StructureLog`]), so it checks what the verification checked. Then it
//! reads the output's pixels of every name part the source showed, not
//! only the parts the boxes leave open. An output on which a name stays
//! readable is renamed `*_UNVERIFIED`.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use blurrd_core::{
    AppError, Coverage, ErrorKind, JobEvent, JobId, JobKind, JobSummary, Project, RegionSet, Stage,
    VerifyReport, VerifyState,
};
use blurrd_video::{
    DecodeOptions, DecodePurpose, Decoder, DecoderChoice, HwDecode, RgbConverter, VideoColour,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::analysis::frame::Cropper;
use crate::analysis::pass0::{Pass0, ZoneReads};
use crate::cache::{self, ProjectDir};
use crate::dictionary::DictionaryBuilder;
use crate::engine::Engine;
use crate::hud::HudMap;
use crate::job::{Job, canceled};
use crate::render::{UNVERIFIED_SUFFIX, check_media, unverified_name};
use crate::verify::{self, Context, HudRead, PixelSetup, Pixels, StructureLog};

/// What to audit.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct AuditRequest {
    /// The job's id (its events carry it).
    pub job: JobId,
    /// The source video.
    pub input: PathBuf,
    /// The rendered output of `project`.
    pub output: PathBuf,
    /// The project the output was rendered with.
    pub project: Project,
    /// Which frames are read: the zones the analysis read on its frames
    /// ([`Coverage::Structure`]) or every frame.
    pub coverage: Coverage,
}

impl AuditRequest {
    /// Audits `output`, rendered from `input` with `project`, reading the
    /// zones the analysis read on its frames.
    pub fn new(job: JobId, input: PathBuf, output: PathBuf, project: Project) -> Self {
        Self {
            job,
            input,
            output,
            project,
            coverage: Coverage::Structure,
        }
    }
}

/// What an audit found.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[non_exhaustive]
pub struct AuditReport {
    /// The report (with the output's SHA-256).
    #[serde(skip)]
    pub report: VerifyReport,
    /// The report file in the cache.
    pub report_file: PathBuf,
    /// The output, renamed `*_UNVERIFIED` when a name stays readable.
    pub output: PathBuf,
    /// Whether the audit renamed the output.
    pub renamed: bool,
    /// Frames read.
    pub frames: u32,
    /// Wall-clock time in milliseconds.
    pub elapsed_ms: u64,
}

impl Engine {
    /// Audits a rendered output: reads the source's protected zones with
    /// OCR, then the output's pixels of every name part found, and lists
    /// every name still readable. An output with a readable name is
    /// renamed `*_UNVERIFIED`. The report is saved in the cache
    /// (`audit-<revision>.json`).
    ///
    /// # Errors
    ///
    /// [`ErrorKind::ProjectMismatch`] when the source is not the project's
    /// or the output has other frames; [`ErrorKind::OcrUnavailable`]
    /// without OCR; decoding, OCR and I/O errors; [`ErrorKind::Canceled`]
    /// when `cancel` fires.
    pub fn audit(
        &self,
        req: &AuditRequest,
        sink: &dyn crate::ProgressSink,
        cancel: &crate::CancellationToken,
    ) -> Result<AuditReport, AppError> {
        let job = Job::new(req.job, sink, cancel);
        let result = self.audit_job(req, &job);
        job.end(result, |done| JobEvent::Finished {
            job: req.job,
            summary: JobSummary {
                frames: done.frames,
                elapsed_ms: done.elapsed_ms,
                output: Some(done.output.clone()),
                verify: Some(done.report.summary.state),
                warnings: job.warnings(),
            },
        })
    }

    fn audit_job(&self, req: &AuditRequest, job: &Job<'_>) -> Result<AuditReport, AppError> {
        job.stage(Stage::Probe);
        let project = &req.project;
        let fingerprint = cache::fingerprint(&req.input)?;
        if !cache::same_file(&project.media.fingerprint, &fingerprint) {
            return Err(mismatch(
                "the source's size or content differs from the analysed video",
            ));
        }
        let info = blurrd_video::probe(&req.input)?;
        let pts = self.pts_table(&req.input, project)?;
        check_media(project, &info, &pts)?;
        let out_info = blurrd_video::probe(&req.output)?;
        let out_pts = Arc::new(blurrd_video::pts_table(&req.output)?);
        if out_pts.len() != pts.len()
            || (out_info.width, out_info.height) != (info.width, info.height)
        {
            return Err(mismatch("the output does not have the source's frames"));
        }
        job.started(JobKind::Audit, pts.len(), 1);
        let config = self.config();
        let setup = config.ocr.as_ref().ok_or_else(|| {
            AppError::new(
                ErrorKind::OcrUnavailable,
                "OCR is not installed, so the output cannot be audited.",
            )
        })?;
        let regions = project.detector.regions;
        let mut pass0 = Pass0::open(
            setup,
            config.adapter,
            &project.geometry,
            &info,
            regions,
            req.coverage,
            1,
            None,
        )
        .map_err(|reason| {
            AppError::new(ErrorKind::OcrUnavailable, "OCR cannot run.").with_detail(reason)
        })?;
        let decoder = |media: &blurrd_core::MediaInfo| {
            let mut decode = DecodeOptions::with_hw(if config.gpu {
                DecoderChoice::resolve(media, &self.caps_dir(), DecodePurpose::SpareCpu)
            } else {
                HwDecode::Software
            });
            decode.threads = config.decoder_threads;
            decode
        };

        job.stage(Stage::Ocr);
        let colour = VideoColour::from_info(&info.color, info.height);
        let map = HudMap::new(&project.geometry, info.width, info.height);
        let mut cropper = Cropper::new(
            map,
            colour,
            RgbConverter::new(colour),
            regions,
            Some(pass0.reads()),
        );
        let dir = ProjectDir::new(&config.cache_dir, &project.media.fingerprint);
        let structure = StructureLog::read(&dir.structure(), pts.len());
        if structure.is_none() {
            tracing::warn!("no structure signals of the analysis; every region counts as on");
        }
        let source = Source {
            input: &req.input,
            decode: decoder(&info),
            coverage: req.coverage,
            structure: structure.as_ref(),
            total: pts.len(),
        };
        let frames = read_source(&source, &mut pass0, &mut cropper, job)?;

        job.stage(Stage::Audit);
        let out_colour = VideoColour::from_info(&out_info.color, out_info.height);
        let pixels = PixelSetup {
            decode: decoder(&out_info),
            blur: None,
            converter: RgbConverter::new(out_colour),
            regions,
            pixel_aspect: project.geometry.sy / project.geometry.sx,
            size: (info.width, info.height),
        };
        let parts = pass0.parts();
        let ctx = Context {
            log: parts.log,
            whitelist: parts.whitelist,
            geometry: &project.geometry,
            size: (info.width, info.height),
            regions,
        };
        let mut reader = Pixels::open(&req.output, out_pts, pixels, parts.ocr)?;
        let outcome = verify::audit(&ctx, &project.plan(), &mut reader, job)?;
        drop(reader);
        self.release(pass0);

        let mut done = self.save_audit(req, &outcome)?;
        done.frames = frames;
        done.elapsed_ms = u64::try_from(job.elapsed().as_millis()).unwrap_or(u64::MAX);
        Ok(done)
    }

    /// Saves the report of `outcome`, the audit of `req`, in the cache and
    /// renames the output when a name stays readable.
    fn save_audit(
        &self,
        req: &AuditRequest,
        outcome: &verify::Outcome,
    ) -> Result<AuditReport, AppError> {
        let project = &req.project;
        let mut report = verify::report(
            outcome,
            project.revision,
            project.media.fingerprint.clone(),
            req.coverage,
        );
        report.output_sha256 = Some(file_sha256(&req.output)?);
        let dir = ProjectDir::new(&self.config().cache_dir, &project.media.fingerprint);
        dir.create()?;
        let report_file = dir
            .project()
            .with_file_name(format!("audit-{}.json", project.revision));
        verify::write_report(&report, &report_file)?;
        let (output, renamed) = name_output(&req.output, report.summary.state)?;
        Ok(AuditReport {
            report,
            report_file,
            output,
            renamed,
            frames: 0,
            elapsed_ms: 0,
        })
    }
}

/// The source video of an audit.
struct Source<'a> {
    input: &'a Path,
    decode: DecodeOptions,
    /// Which frames' HUD zones are read.
    coverage: Coverage,
    /// The structure signals of its analysis, when the cache holds them.
    structure: Option<&'a StructureLog>,
    /// Its frames.
    total: u32,
}

/// Reads `source` with `pass0` (crops cut by `cropper`): every zone of
/// every frame with [`Coverage::EveryFrame`]; otherwise the zones its
/// analysis read on each frame, or, when the cache does not record them,
/// the HUD zones of the frames [`DictionaryBuilder::wants`] and the screen
/// zones and the full frame where the crops hold them. Returns the frames
/// read.
fn read_source(
    source: &Source<'_>,
    pass0: &mut Pass0,
    cropper: &mut Cropper,
    job: &Job<'_>,
) -> Result<u32, AppError> {
    // Exposures of one name join into runs as in the analysis it follows.
    let spacing = source.structure.and_then(StructureLog::hud_spacing);
    if let Some(spacing) = spacing.filter(|_| source.coverage == Coverage::Structure) {
        pass0.set_hud_spacing(spacing);
    }
    let mut decoder = Decoder::open(source.input, &source.decode)?;
    let stopped = || job.is_canceled();
    let mut frames = 0u32;
    while let Some(frame) = decoder.next_frame()? {
        let index = frame.index();
        // The analysis' structure signals gate the slots as they did in
        // its verification. Without them (an analysis without OCR) every
        // region counts as showing its structure; the plan's boxes then
        // confirm the slots alone.
        let (structure, on) = match source.structure {
            Some(log) => {
                let on = log.at(index);
                (on, !on.is_empty())
            }
            None => (RegionSet::all(), false),
        };
        let reads = if source.coverage == Coverage::EveryFrame {
            ZoneReads::ALL
        } else if let Some(reads) = source.structure.and_then(|log| log.reads_at(index)) {
            reads
        } else if DictionaryBuilder::wants(index, on) {
            ZoneReads::ALL
        } else {
            ZoneReads::with_hud(HudRead::NONE)
        };
        let crops = cropper.crops(frame)?;
        let read = pass0
            .read(&crops, reads, structure, &stopped)
            .map_err(|reason| {
                AppError::new(ErrorKind::OcrUnavailable, "OCR stopped.").with_detail(reason)
            })?;
        if !read {
            return Err(canceled());
        }
        frames += 1;
        job.progress(frames, source.total.max(frames));
    }
    Ok(frames)
}

/// The output's name after an audit with `state`: renamed `*_UNVERIFIED`
/// when a name stays readable (unless it is already named so). Returns
/// the name and whether it changed.
fn name_output(output: &Path, state: VerifyState) -> Result<(PathBuf, bool), AppError> {
    let already = output
        .file_stem()
        .is_some_and(|s| s.to_string_lossy().ends_with(UNVERIFIED_SUFFIX));
    if state != VerifyState::LeaksRemain || already {
        return Ok((output.to_path_buf(), false));
    }
    let renamed = unverified_name(output);
    std::fs::rename(output, &renamed)
        .map_err(|e| cache::io_error(&e, "The output could not be renamed.", output))?;
    Ok((renamed, true))
}

/// The hex SHA-256 of the file at `path`.
fn file_sha256(path: &Path) -> Result<String, AppError> {
    let mut file = std::fs::File::open(path)
        .map_err(|e| cache::io_error(&e, "The output could not be read.", path))?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|e| cache::io_error(&e, "The output could not be read.", path))?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Ok(hash
        .finalize()
        .iter()
        .fold(String::with_capacity(64), |mut hex, b| {
            use std::fmt::Write as _;
            let _ = write!(hex, "{b:02x}");
            hex
        }))
}

/// A project mismatch with `detail`.
fn mismatch(detail: &str) -> AppError {
    AppError::new(
        ErrorKind::ProjectMismatch,
        "The video is not the one the project was made for.",
    )
    .with_detail(detail)
}
