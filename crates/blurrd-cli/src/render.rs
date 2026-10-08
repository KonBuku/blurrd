//! `blurrd render`: blur a recording with an analysed project.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use blurrd_blur::{CpuBlur, PixelParams};
use blurrd_core::{
    AppError, EncoderSetting, ErrorKind, ExportSettings, JobEvent, JobKind, JobSummary, MediaInfo,
    Project, Stage, VerifyState, WarningCode, schema,
};
use blurrd_video::{
    DecodeOptions, DecodePurpose, DecoderChoice, EncodeOptions, HwDecode, MoovLayout, MuxOptions,
    PtsTable, TranscodeReport, Transcoder, VideoColour, VideoError,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::args::{Global, RenderArgs};
use crate::output::Events;

/// How a render ended (errors are returned separately).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// The output was written; `unverified` when it is named
    /// `*_UNVERIFIED` because the project is not verified.
    Rendered { unverified: bool },
    /// The user pressed Ctrl+C; no output was left behind.
    Canceled,
}

/// The final report line of a render.
#[derive(Debug, Serialize)]
struct RenderReport {
    output: PathBuf,
    frames: u32,
    blurred_frames: u32,
    encoder: String,
    sessions: u8,
    elapsed_ms: u64,
    fps: f64,
    /// Whether the project was verified (or the user accepted it).
    verified: bool,
    fallbacks: Vec<Fallback>,
    skipped_streams: Vec<Skipped>,
    /// Where the MP4/MOV index is: `reserved`, `faststart`, `end` or
    /// `none` (not MP4/MOV).
    moov: &'static str,
}

#[derive(Debug, Serialize)]
struct Fallback {
    from: String,
    to: String,
    reason: String,
}

#[derive(Debug, Serialize)]
struct Skipped {
    index: u32,
    codec: String,
    reason: String,
}

/// Renders `args.input` with the boxes of the project `args.plan`.
pub(crate) fn render(
    args: &RenderArgs,
    global: &Global,
    events: &mut Events,
    cancel: &AtomicBool,
) -> Result<Outcome, AppError> {
    let job = events.job();
    let project = read_project(&args.plan)?;
    blurrd_video::init()?;
    events.emit(&JobEvent::Stage {
        job,
        stage: Stage::Probe,
    });
    let caps = cache_dir().join("caps");
    // The encoder probe takes a second on its first run; it overlaps with
    // probing the input.
    let wants_probe = args.encoder == EncoderSetting::Auto && !global.no_gpu;
    let encoder_probe = wants_probe.then(|| {
        let caps = caps.clone();
        std::thread::spawn(move || blurrd_video::probe_encoders(&caps))
    });
    let info = blurrd_video::probe(&args.input)?;
    let pts = blurrd_video::pts_table(&args.input)?;
    check_media(&project, &info, &pts)?;

    let verified = is_verified(&project) || args.accept_unverified;
    let output = if verified {
        args.output.clone()
    } else {
        unverified_name(&args.output)
    };
    let export = export_settings(args)?;
    let mut encode = EncodeOptions::from_export(&export);
    encode.gpu = !global.no_gpu;
    if let Some(probe) = encoder_probe {
        encode.probed = probe.join().unwrap_or_default();
    }
    let mut decode = DecodeOptions::with_hw(if global.no_gpu {
        HwDecode::Software
    } else {
        DecoderChoice::resolve(&info, &caps, DecodePurpose::Throughput)
    });
    decode.threads = global.threads;

    let mut blur = project.blur;
    if let Some(strength) = args.strength {
        blur.strength = strength.into();
    }
    let colour = VideoColour::from_info(&info.color, info.height);
    let params =
        PixelParams::new(&blur, project.geometry.sy, colour.yuv_colour()).map_err(|e| {
            AppError::new(
                ErrorKind::InvalidProject,
                "The project's blur settings are invalid.",
            )
            .with_detail(e.to_string())
        })?;
    let table = project.plan().frame_table(&project.detector.regions);

    let total = pts.len();
    events.emit(&JobEvent::Started {
        job,
        kind: JobKind::Render,
        total_frames: total,
    });
    let transcoder = Transcoder::open(
        &info,
        Arc::new(pts),
        &output,
        &decode,
        &encode,
        MuxOptions::from_export(&export),
    )?;
    for fallback in transcoder.fallbacks() {
        events.warning(
            WarningCode::EncoderFallback,
            format!(
                "{} could not be used ({}); encoding with {}",
                fallback.from, fallback.reason, fallback.to
            ),
        );
    }
    events.emit(&JobEvent::Stage {
        job,
        stage: Stage::BlurEncode,
    });
    let stop = || cancel.load(Ordering::Relaxed);
    let report = match transcoder.run(&table, &CpuBlur, &params, &stop, |n| {
        events.progress(n, total);
    }) {
        Ok(report) => report,
        Err(VideoError::Canceled) => {
            events.emit(&JobEvent::Canceled { job });
            return Ok(Outcome::Canceled);
        }
        Err(error) => return Err(error.into()),
    };
    for skipped in &report.skipped_streams {
        tracing::warn!(stream = skipped.index, codec = %skipped.codec, reason = %skipped.reason,
            "stream not in the output");
    }
    finish(events, &report, &project, verified);
    Ok(Outcome::Rendered {
        unverified: !verified,
    })
}

/// Emits the `Finished` event and the report.
fn finish(events: &mut Events, report: &TranscodeReport, project: &Project, verified: bool) {
    let elapsed_ms = u64::try_from(report.elapsed.as_millis()).unwrap_or(u64::MAX);
    events.emit(&JobEvent::Finished {
        job: events.job(),
        summary: JobSummary {
            frames: report.frames,
            elapsed_ms,
            output: Some(report.output.clone()),
            verify: Some(if verified {
                VerifyState::Clean
            } else {
                project.verify.state
            }),
            warnings: events.warnings(),
        },
    });
    let fps = f64::from(report.frames) / report.elapsed.as_secs_f64().max(1e-9);
    let line = RenderReport {
        output: report.output.clone(),
        frames: report.frames,
        blurred_frames: report.blurred_frames,
        encoder: report.encoder.clone(),
        sessions: report.sessions,
        elapsed_ms,
        fps,
        verified,
        fallbacks: report
            .fallbacks
            .iter()
            .map(|f| Fallback {
                from: f.from.clone(),
                to: f.to.clone(),
                reason: f.reason.clone(),
            })
            .collect(),
        skipped_streams: report
            .skipped_streams
            .iter()
            .map(|s| Skipped {
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
    };
    let mut summary = format!(
        "wrote {} ({} frames, {} blurred, {} x{}, {:.1} s, {fps:.0} fps)",
        report.output.display(),
        report.frames,
        report.blurred_frames,
        report.encoder,
        report.sessions,
        report.elapsed.as_secs_f64(),
    );
    if !verified {
        summary.push_str(
            "\nthe project is not verified: check the output before publishing it, \
             or pass --accept-unverified",
        );
    }
    events.report(&line, &summary);
}

/// Reads and validates the project file.
fn read_project(path: &Path) -> Result<Project, AppError> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        let kind = if e.kind() == std::io::ErrorKind::NotFound {
            ErrorKind::NotFound
        } else {
            ErrorKind::Io
        };
        AppError::new(kind, "The project file could not be read.")
            .with_detail(format!("{}: {e}", path.display()))
    })?;
    Ok(schema::from_json::<Project>(&text)?)
}

/// Checks that the input is the video the project was made for: same
/// picture size, same frames with the same timestamps.
fn check_media(project: &Project, info: &MediaInfo, pts: &PtsTable) -> Result<(), AppError> {
    let media = &project.media;
    let mismatch = |detail: String| {
        AppError::new(
            ErrorKind::ProjectMismatch,
            "The video is not the one the project was made for.",
        )
        .with_detail(detail)
    };
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
    if !media.pts_sha256.eq_ignore_ascii_case(&pts_sha256(pts)) {
        return Err(mismatch(String::from(
            "the frame timestamps differ from the project's",
        )));
    }
    Ok(())
}

/// The project's `pts_sha256`: the hex SHA-256 of the PTS table in the
/// `pts.bin` format.
fn pts_sha256(pts: &PtsTable) -> String {
    use std::fmt::Write as _;
    Sha256::digest(pts.to_bytes())
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

/// Whether the project's last verification is clean and current.
fn is_verified(project: &Project) -> bool {
    project.verify.state == VerifyState::Clean
        && project.verify.plan_revision == project.revision
        && project.verify.unverified_ranges.is_empty()
}

/// `name_UNVERIFIED.ext` next to `name.ext`.
fn unverified_name(output: &Path) -> PathBuf {
    let stem = output.file_stem().unwrap_or_default().to_string_lossy();
    let name = match output.extension() {
        Some(ext) => format!("{stem}_UNVERIFIED.{}", ext.to_string_lossy()),
        None => format!("{stem}_UNVERIFIED"),
    };
    output.with_file_name(name)
}

/// The export settings the arguments describe.
fn export_settings(args: &RenderArgs) -> Result<ExportSettings, AppError> {
    let export = ExportSettings {
        codec: args.codec.into(),
        encoder: args.encoder.clone(),
        rate: args.quality,
        preset: args.preset,
        nvenc_sessions: args.sessions,
        audio: args.audio,
        ..ExportSettings::default()
    };
    match export.problem() {
        Some(problem) => {
            Err(AppError::new(ErrorKind::Internal, "Invalid export settings.").with_detail(problem))
        }
        None => Ok(export),
    }
}

/// blurrd's cache folder: `%LOCALAPPDATA%\blurrd` (the system temporary
/// folder if that is not set).
fn cache_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map_or_else(std::env::temp_dir, PathBuf::from)
        .join("blurrd")
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use blurrd_core::Rational;
    use blurrd_video::PtsTable;

    use super::{pts_sha256, unverified_name};

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

    #[test]
    fn the_pts_hash_covers_the_pts_bin_bytes() {
        let table = PtsTable::new(Rational::new(1, 60).unwrap(), vec![0, 1, 2]).unwrap();
        let hash = pts_sha256(&table);
        assert_eq!(hash.len(), 64);
        let other = PtsTable::new(Rational::new(1, 60).unwrap(), vec![0, 1, 3]).unwrap();
        assert_ne!(pts_sha256(&other), hash);
    }
}
