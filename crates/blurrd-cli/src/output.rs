//! What the CLI prints: job events and reports as JSON lines on stdout
//! with `--json`, or a progress line on stderr and a short summary on
//! stdout for people.

use std::io::Write;
use std::time::{Duration, Instant};

use blurrd_core::{AppError, JobEvent, JobId, WarningCode};
use serde::Serialize;

/// Progress events and lines are sent at most this often (20 per second).
const PROGRESS_INTERVAL: Duration = Duration::from_millis(50);

/// Writes the events of one job.
#[derive(Debug)]
pub(crate) struct Events {
    json: bool,
    job: JobId,
    started: Instant,
    last_progress: Option<Instant>,
    /// Whether a progress line is on the terminal (and needs ending).
    progress_line: bool,
    warnings: u32,
}

impl Events {
    /// Events of the job `job`; JSON lines when `json` is set.
    pub(crate) fn new(job: JobId, json: bool) -> Self {
        Self {
            json,
            job,
            started: Instant::now(),
            last_progress: None,
            progress_line: false,
            warnings: 0,
        }
    }

    /// The job the events belong to.
    pub(crate) fn job(&self) -> JobId {
        self.job
    }

    /// Number of warnings emitted so far.
    pub(crate) fn warnings(&self) -> u32 {
        self.warnings
    }

    /// Emits an event: as a JSON line, or (for warnings and failures) as a
    /// line on stderr.
    pub(crate) fn emit(&mut self, event: &JobEvent) {
        if let JobEvent::Warning { .. } = event {
            self.warnings += 1;
        }
        if self.json {
            write_json(event);
            return;
        }
        match event {
            JobEvent::Warning { message, .. } => self.line(&format!("warning: {message}")),
            JobEvent::Canceled { .. } => self.line("canceled"),
            _ => {}
        }
    }

    /// Emits a warning.
    pub(crate) fn warning(&mut self, code: WarningCode, message: String) {
        let event = JobEvent::Warning {
            job: self.job,
            code,
            message,
            frames: None,
        };
        self.emit(&event);
    }

    /// Reports that `frame` of `total` frames are done, at most 20 times
    /// per second (and always for the last frame).
    pub(crate) fn progress(&mut self, frame: u32, total: u32) {
        let now = Instant::now();
        let due = self
            .last_progress
            .is_none_or(|last| now.duration_since(last) >= PROGRESS_INTERVAL);
        if !due && frame < total {
            return;
        }
        self.last_progress = Some(now);
        let seconds = self.started.elapsed().as_secs_f64();
        let fps = if seconds > 0.0 {
            f64::from(frame) / seconds
        } else {
            0.0
        };
        let eta_ms = if fps > 0.0 {
            (f64::from(total.saturating_sub(frame)) / fps * 1000.0) as u64
        } else {
            0
        };
        if self.json {
            write_json(&JobEvent::Progress {
                job: self.job,
                frame,
                total,
                fps: fps as f32,
                eta_ms,
            });
        } else {
            let mut stderr = std::io::stderr().lock();
            // A failed write to the terminal is not worth failing the job.
            let _ = write!(
                stderr,
                "\rrender: {frame}/{total} frames, {fps:.0} fps, {:.1} s left   ",
                eta_ms as f64 / 1000.0
            );
            let _ = stderr.flush();
            self.progress_line = true;
        }
    }

    /// Reports a failure: a `Failed` event with `--json`, a message on
    /// stderr otherwise.
    pub(crate) fn failed(&mut self, error: &AppError) {
        if self.json {
            write_json(&JobEvent::Failed {
                job: self.job,
                error: error.clone(),
            });
        } else {
            self.line(&error_text(error));
        }
    }

    /// The final report: a JSON line `{"type":"Report", …}`, or `summary`
    /// on stdout.
    pub(crate) fn report(&mut self, report: &impl Serialize, summary: &str) {
        if self.json {
            write_json(&Tagged {
                kind: "Report",
                body: report,
            });
        } else {
            self.end_progress();
            let _ = writeln!(std::io::stdout().lock(), "{summary}");
        }
    }

    /// Writes a line on stderr, ending a progress line first.
    fn line(&mut self, text: &str) {
        self.end_progress();
        let _ = writeln!(std::io::stderr().lock(), "{text}");
    }

    fn end_progress(&mut self) {
        if std::mem::take(&mut self.progress_line) {
            let _ = writeln!(std::io::stderr().lock());
        }
    }
}

/// A JSON object with a `type` field next to the fields of `body`.
#[derive(Serialize)]
struct Tagged<'a, T: Serialize> {
    #[serde(rename = "type")]
    kind: &'static str,
    #[serde(flatten)]
    body: &'a T,
}

/// Writes one JSON line on stdout.
pub(crate) fn write_json(value: &impl Serialize) {
    match serde_json::to_string(value) {
        Ok(line) => {
            let mut stdout = std::io::stdout().lock();
            // A closed pipe (the reader went away) is not an error of ours.
            let _ = writeln!(stdout, "{line}");
            let _ = stdout.flush();
        }
        Err(error) => tracing::error!(%error, "could not serialise output"),
    }
}

/// Writes an error that is not part of a job (for example from `probe`):
/// `{"type":"Error","error":…}` with `--json`, a message on stderr
/// otherwise.
pub(crate) fn write_error(error: &AppError, json: bool) {
    #[derive(Serialize)]
    struct ErrorLine<'a> {
        error: &'a AppError,
    }
    if json {
        write_json(&Tagged {
            kind: "Error",
            body: &ErrorLine { error },
        });
    } else {
        let _ = writeln!(std::io::stderr().lock(), "{}", error_text(error));
    }
}

/// `error: <message> (<detail>)`.
fn error_text(error: &AppError) -> String {
    match &error.detail {
        Some(detail) => format!("error: {} ({detail})", error.message),
        None => format!("error: {}", error.message),
    }
}
