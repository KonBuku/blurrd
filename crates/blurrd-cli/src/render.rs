//! `blurrd render`: blur a recording with an analysed project.

use blurrd_core::{AppError, ErrorKind, ExportSettings, JobId};
use blurrd_engine::{CancellationToken, Engine, RenderReport, RenderRequest, cache};

use crate::Outcome;
use crate::args::{ExportArgs, RenderArgs};
use crate::output::Events;

/// Renders `args.input` with the boxes of the project `args.plan`.
pub(crate) fn render(
    args: &RenderArgs,
    engine: &Engine,
    events: &Events,
    cancel: &CancellationToken,
) -> Outcome {
    let job = JobId(uuid::Uuid::now_v7());
    let prepared = cache::read_project(&args.plan).and_then(|mut project| {
        if let Some(strength) = args.export.strength {
            project.blur.strength = strength.into();
        }
        Ok((project, export_settings(&args.export)?))
    });
    let (project, export) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => {
            events.failed(job, &error);
            return Outcome::Failed;
        }
    };
    let mut req = RenderRequest::new(job, args.input.clone(), project, args.output.clone());
    req.export = export;
    req.accept_unverified = args.export.accept_unverified;
    match engine.render(&req, events, cancel) {
        Ok(report) => {
            events.report(&report, &summary(&report));
            Outcome::Done {
                unverified: !report.verified,
            }
        }
        Err(error) => Outcome::from_error(&error),
    }
}

/// The summary line of a render for people.
pub(crate) fn summary(report: &RenderReport) -> String {
    let mut summary = format!(
        "wrote {} ({} frames, {} blurred, {} x{}, {:.1} s, {:.0} fps)",
        report.output.display(),
        report.frames,
        report.blurred_frames,
        report.encoder,
        report.sessions,
        report.elapsed_ms as f64 / 1000.0,
        report.fps,
    );
    if !report.verified {
        summary.push_str(
            "\nthe result is not verified: check the output before publishing it, \
             or pass --accept-unverified",
        );
    }
    summary
}

/// The export settings the arguments describe.
pub(crate) fn export_settings(args: &ExportArgs) -> Result<ExportSettings, AppError> {
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
        Some(problem) => Err(AppError::new(
            ErrorKind::InvalidProject,
            "The export settings are invalid.",
        )
        .with_detail(problem)),
        None => Ok(export),
    }
}
