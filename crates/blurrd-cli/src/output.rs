//! What the CLI prints: job events and reports as JSON lines on stdout
//! with `--json`, or a progress line on stderr and a short summary on
//! stdout for people.

use std::io::{IsTerminal, Write};
use std::sync::Mutex;

use blurrd_core::{AppError, JobEvent, JobId, JobKind};
use blurrd_engine::ProgressSink;
use serde::Serialize;

/// Receives the events of the engine's jobs and prints them.
#[derive(Debug)]
pub(crate) struct Events {
    json: bool,
    /// Whether stderr is a terminal (a progress line is drawn only there).
    terminal: bool,
    state: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    /// What the progress line calls the running job.
    label: &'static str,
    /// Whether a progress line is on the terminal (and needs ending).
    progress_line: bool,
}

impl Events {
    /// Events printed as JSON lines when `json` is set.
    pub(crate) fn new(json: bool) -> Self {
        Self {
            json,
            terminal: std::io::stderr().is_terminal(),
            state: Mutex::new(State {
                label: "job",
                progress_line: false,
            }),
        }
    }

    /// Reports a failure of job `job` that happened before the engine
    /// started it (the engine reports its own failures): a `Failed` event
    /// with `--json`, a message on stderr otherwise.
    pub(crate) fn failed(&self, job: JobId, error: &AppError) {
        self.emit(JobEvent::Failed {
            job,
            error: error.clone(),
        });
    }

    /// The final report: a JSON line `{"type":"Report", …}`, or `summary`
    /// on stdout.
    pub(crate) fn report(&self, report: &impl Serialize, summary: &str) {
        if self.json {
            write_json(&Tagged {
                kind: "Report",
                body: report,
            });
        } else {
            self.with_state(State::end_progress);
            let _ = writeln!(std::io::stdout().lock(), "{summary}");
        }
    }

    fn with_state(&self, f: impl FnOnce(&mut State)) {
        if let Ok(mut state) = self.state.lock() {
            f(&mut state);
        }
    }
}

impl ProgressSink for Events {
    /// Prints an event: every event as a JSON line with `--json`;
    /// otherwise progress as one updating line on stderr when that is a
    /// terminal, warnings, failures and cancellations as lines on stderr.
    fn emit(&self, event: JobEvent) {
        if self.json {
            write_json(&event);
            return;
        }
        self.with_state(|state| match &event {
            JobEvent::Started { kind, .. } => {
                state.label = match kind {
                    JobKind::Analyze => "analyze",
                    JobKind::Render => "render",
                    JobKind::Run => "run",
                    _ => "job",
                };
            }
            JobEvent::Progress {
                frame,
                total,
                fps,
                eta_ms,
                ..
            } if self.terminal => {
                let mut stderr = std::io::stderr().lock();
                // A failed write to the terminal is not worth failing the
                // job.
                let _ = write!(
                    stderr,
                    "\r{}: {frame}/{total} frames, {fps:.0} fps, {:.1} s left   ",
                    state.label,
                    *eta_ms as f64 / 1000.0
                );
                let _ = stderr.flush();
                state.progress_line = true;
            }
            JobEvent::Warning { message, .. } => state.line(&format!("warning: {message}")),
            JobEvent::Failed { error, .. } => state.line(&error_text(error)),
            JobEvent::Canceled { .. } => state.line("canceled"),
            _ => {}
        });
    }
}

impl State {
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
