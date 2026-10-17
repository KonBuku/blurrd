//! `blurrd analyze`: find the names in a recording and save the project.

use std::path::PathBuf;

use blurrd_core::JobId;
use blurrd_engine::{AnalyzeReport, AnalyzeRequest, CancellationToken, Engine, cache};
use serde::Serialize;

use crate::Outcome;
use crate::args::AnalyzeArgs;
use crate::output::{Events, leak_report_file, verify_line};

/// The final report line of an analysis.
#[derive(Debug, Serialize)]
struct Report<'a> {
    #[serde(flatten)]
    analysis: &'a AnalyzeReport,
    /// The exported project file, when `-o` was given.
    plan: Option<&'a PathBuf>,
}

/// Analyses `args.input`; the project goes into the cache and, with
/// `-o`, into a file.
pub(crate) fn analyze(
    args: &AnalyzeArgs,
    engine: &Engine,
    events: &Events,
    cancel: &CancellationToken,
) -> Outcome {
    let job = JobId(uuid::Uuid::now_v7());
    let mut req = AnalyzeRequest::new(job, args.input.clone());
    req.regions = args.detect.regions;
    req.verify = !args.detect.no_verify;
    // Thumbnails and the preview proxy are for the desktop app.
    req.previews = false;
    let analysis = match engine.analyze(&req, events, cancel) {
        Ok(analysis) => analysis,
        Err(error) => return Outcome::from_error(&error),
    };
    if let Some(output) = &args.output
        && let Err(error) = cache::write_project(&analysis.project, output)
    {
        events.failed(job, &error);
        return Outcome::Failed;
    }
    let report = &analysis.report;
    let mut summary = format!(
        "analysed {}: {} frames, {} tracks, {:.1} s, {:.0} fps\nproject: {}",
        args.input.display(),
        report.frames,
        report.tracks,
        report.elapsed_ms as f64 / 1000.0,
        report.fps,
        args.output
            .as_ref()
            .unwrap_or(&report.project_file)
            .display(),
    );
    match (&report.verify, &report.verify_report) {
        (Some(verify), Some(path)) => {
            summary.push('\n');
            summary.push_str(&leak_report_file(path, verify));
        }
        (Some(verify), None) => {
            summary.push('\n');
            summary.push_str(&verify_line(verify));
        }
        (None, _) => summary.push_str("\nnot verified (no OCR)"),
    }
    events.report(
        &Report {
            analysis: report,
            plan: args.output.as_ref(),
        },
        &summary,
    );
    Outcome::Done { unverified: false }
}
