//! `blurrd run`: analyse and blur recordings in one pass.

use std::path::{Path, PathBuf};

use blurrd_core::{AppError, BlurParams, ErrorKind, JobId};
use blurrd_engine::{CancellationToken, Engine, RunReport, RunRequest};
use serde::Serialize;

use crate::Outcome;
use crate::args::RunArgs;
use crate::output::Events;
use crate::render::{export_settings, summary};

/// Containers an output keeps from its input; other inputs become MP4.
const CONTAINERS: [&str; 3] = ["mp4", "mov", "mkv"];

/// The final report line of a run: one entry per recording that was
/// written.
#[derive(Debug, Serialize)]
struct Report<'a> {
    runs: &'a [RunReport],
}

/// Runs every input of `args`, one job each, until one is canceled.
pub(crate) fn run(
    args: &RunArgs,
    engine: &Engine,
    events: &Events,
    cancel: &CancellationToken,
) -> Outcome {
    let several = args.inputs.len() > 1;
    if several && let Err(error) = std::fs::create_dir_all(&args.output) {
        let error = AppError::new(ErrorKind::Io, "The output folder could not be created.")
            .with_detail(format!("{}: {error}", args.output.display()));
        events.failed(JobId(uuid::Uuid::now_v7()), &error);
        return Outcome::Failed;
    }
    let export = match export_settings(&args.export) {
        Ok(export) => export,
        Err(error) => {
            events.failed(JobId(uuid::Uuid::now_v7()), &error);
            return Outcome::Failed;
        }
    };
    let mut blur = BlurParams::default();
    if let Some(strength) = args.export.strength {
        blur.strength = strength.into();
    }
    let mut reports = Vec::new();
    let mut outcome = Outcome::Done { unverified: false };
    for input in &args.inputs {
        let job = JobId(uuid::Uuid::now_v7());
        let output = output_path(input, &args.output, several);
        let mut req = RunRequest::new(job, input.clone(), output);
        req.regions = args.detect.regions;
        req.verify = !args.detect.no_verify;
        req.blur = blur;
        req.export = export.clone();
        req.accept_unverified = args.export.accept_unverified;
        match engine.run(&req, events, cancel) {
            Ok(report) => {
                outcome = outcome.and(Outcome::Done {
                    unverified: !report.render.verified,
                });
                reports.push(report);
            }
            Err(error) => {
                outcome = outcome.and(Outcome::from_error(&error));
                if outcome == Outcome::Canceled {
                    break;
                }
            }
        }
    }
    let summaries: Vec<String> = reports.iter().map(|r| summary(&r.render)).collect();
    events.report(&Report { runs: &reports }, &summaries.join("\n"));
    outcome
}

/// Where the output of `input` goes: `output` itself for one recording
/// unless it is a folder; otherwise `<name>_blurrd.<ext>` in that folder,
/// keeping MP4, MOV and Matroska containers.
fn output_path(input: &Path, output: &Path, several: bool) -> PathBuf {
    if !several && !output.is_dir() {
        return output.to_path_buf();
    }
    let stem = input.file_stem().unwrap_or_default().to_string_lossy();
    let extension = input
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .filter(|e| CONTAINERS.contains(&e.as_str()))
        .unwrap_or_else(|| String::from("mp4"));
    output.join(format!("{stem}_blurrd.{extension}"))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::output_path;

    #[test]
    fn outputs_go_into_folders_with_the_input_name() {
        let dir = std::env::temp_dir();
        assert_eq!(
            output_path(Path::new("a/clip.MKV"), &dir, false),
            dir.join("clip_blurrd.mkv")
        );
        assert_eq!(
            output_path(Path::new("clip.avi"), Path::new("out"), true),
            Path::new("out").join("clip_blurrd.mp4")
        );
        assert_eq!(
            output_path(Path::new("clip.mp4"), Path::new("x/done.mov"), false),
            Path::new("x/done.mov")
        );
    }
}
