//! `blurrd`, the command-line tool.
//!
//! Commands: `probe` shows what blurrd knows about a recording, `analyze`
//! finds the names, verifies the blurred result with OCR and saves the
//! project, `render` blurs a recording with an analysed project, `run`
//! does both in one streaming pass, `audit` reads a rendered output with
//! OCR. With `--json`, every job event is one JSON line on stdout (the
//! same form the desktop app receives), followed by a final
//! `{"type":"Report", …}` line. The leak report lists frames, regions,
//! boxes and states, never text.
//!
//! Exit codes: 0 success, 1 error, 2 invalid arguments, 3 the output was
//! written but is not verified (named `*_UNVERIFIED`) or the audit found a
//! readable name, 130 canceled with Ctrl+C.

#![forbid(unsafe_code)]

mod analyze;
mod args;
mod audit;
mod output;
mod probe;
mod render;
mod run;

use std::path::PathBuf;
use std::process::ExitCode;

use blurrd_core::{AppError, ErrorKind};
use blurrd_engine::{CancellationToken, Engine, EngineConfig, OcrSetup, default_cache_dir};
use clap::Parser;
use tracing_subscriber::EnvFilter;

use crate::args::{Cli, Command, Global};
use crate::output::{Events, write_error};

/// Exit code for a written but unverified output.
const EXIT_UNVERIFIED: u8 = 3;
/// Exit code after Ctrl+C (128 + SIGINT, as shells report it).
const EXIT_CANCELED: u8 = 130;

/// How a job command ended. Failures are reported where they happen (by
/// the engine's events, or by the command before the engine ran).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// Finished; `unverified` when an output is named `*_UNVERIFIED`.
    Done { unverified: bool },
    /// The user pressed Ctrl+C; no output was left behind.
    Canceled,
    /// A job failed.
    Failed,
}

impl Outcome {
    /// The outcome of a job that ended with `error`.
    pub(crate) fn from_error(error: &AppError) -> Self {
        if error.kind == ErrorKind::Canceled {
            Self::Canceled
        } else {
            Self::Failed
        }
    }

    /// The outcome of two jobs: cancellation over failure over an
    /// unverified output over success.
    pub(crate) fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::Canceled, _) | (_, Self::Canceled) => Self::Canceled,
            (Self::Failed, _) | (_, Self::Failed) => Self::Failed,
            (Self::Done { unverified: a }, Self::Done { unverified: b }) => {
                Self::Done { unverified: a || b }
            }
        }
    }

    fn exit_code(self) -> ExitCode {
        match self {
            Self::Done { unverified: false } => ExitCode::SUCCESS,
            Self::Done { unverified: true } => ExitCode::from(EXIT_UNVERIFIED),
            Self::Canceled => ExitCode::from(EXIT_CANCELED),
            Self::Failed => ExitCode::FAILURE,
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    init_logging(&cli.global);
    match &cli.command {
        Command::Probe(args) => match probe::probe(args, cli.global.json) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                write_error(&error, cli.global.json);
                ExitCode::FAILURE
            }
        },
        command => job(command, &cli.global).exit_code(),
    }
}

/// Runs a job command with an engine; Ctrl+C cancels the job.
fn job(command: &Command, global: &Global) -> Outcome {
    let cancel = CancellationToken::new();
    let on_ctrl_c = cancel.clone();
    if let Err(error) = ctrlc::set_handler(move || on_ctrl_c.cancel()) {
        tracing::warn!(%error, "Ctrl+C will not cancel cleanly");
    }
    let engine = match Engine::new(engine_config(global)) {
        Ok(engine) => engine,
        Err(error) => {
            write_error(&error, global.json);
            return Outcome::Failed;
        }
    };
    let events = Events::new(global.json);
    match command {
        Command::Analyze(args) => analyze::analyze(args, &engine, &events, &cancel),
        Command::Render(args) => render::render(args, &engine, &events, &cancel),
        Command::Run(args) => run::run(args, &engine, &events, &cancel),
        Command::Audit(args) => audit::audit(args, &engine, &events, &cancel),
        // Probing needs no job; `main` handles it.
        Command::Probe(_) => Outcome::Done { unverified: false },
    }
}

/// The engine configuration of the global options: the cache in
/// `%LOCALAPPDATA%\blurrd`, OCR when ONNX Runtime and the models are
/// installed.
fn engine_config(global: &Global) -> EngineConfig {
    let mut config = EngineConfig::new(default_cache_dir());
    config.gpu = !global.no_gpu;
    config.decoder_threads = global.threads;
    config.ocr = ocr_setup().map(|mut setup| {
        setup.gpu = !global.no_gpu;
        setup
    });
    config
}

/// ONNX Runtime and the OCR models: `ORT_DYLIB_PATH` and
/// `BLURRD_MODEL_DIR` when set (development), otherwise `onnxruntime.dll`
/// and `models` next to the executable (installed); `None` when either is
/// missing.
fn ocr_setup() -> Option<OcrSetup> {
    let next_to_exe = |name: &str| {
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.join(name)))
    };
    let from_env = |var: &str| std::env::var_os(var).map(PathBuf::from);
    let dylib = from_env("ORT_DYLIB_PATH").or_else(|| next_to_exe("onnxruntime.dll"))?;
    let models = from_env("BLURRD_MODEL_DIR").or_else(|| next_to_exe("models"))?;
    (dylib.is_file() && models.is_dir()).then(|| OcrSetup::new(dylib, models))
}

/// Logs to stderr at the requested level (`RUST_LOG` overrides it), as
/// JSON lines with `--json`.
fn init_logging(global: &Global) {
    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(&global.log_level))
        .unwrap_or_else(|_| EnvFilter::new("warn"));
    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr);
    let installed = if global.json {
        builder.json().try_init()
    } else {
        builder.with_target(false).try_init()
    };
    if installed.is_err() {
        write_error(
            &AppError::new(ErrorKind::Internal, "Logging could not be set up."),
            global.json,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::Outcome;

    #[test]
    fn outcomes_combine_by_severity() {
        let ok = Outcome::Done { unverified: false };
        let unverified = Outcome::Done { unverified: true };
        assert_eq!(ok.and(unverified), unverified);
        assert_eq!(unverified.and(Outcome::Failed), Outcome::Failed);
        assert_eq!(Outcome::Failed.and(Outcome::Canceled), Outcome::Canceled);
        assert_eq!(ok.and(ok), ok);
    }
}
