//! `blurrd`, the command-line tool.
//!
//! Commands: `probe` shows what blurrd knows about a recording, `render`
//! blurs it with an analysed project and exports it. With `--json`, every
//! job event is one JSON line on stdout (the same form the desktop app
//! receives), followed by a final `{"type":"Report", …}` line.
//!
//! Exit codes: 0 success, 1 error, 2 invalid arguments, 3 the output was
//! written but is not verified (named `*_UNVERIFIED`), 130 canceled with
//! Ctrl+C.

#![forbid(unsafe_code)]

mod args;
mod output;
mod probe;
mod render;

use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};

use clap::Parser;
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

use crate::args::{Cli, Command, Global};
use crate::output::{Events, write_error};
use crate::render::Outcome;

/// Exit code for a written but unverified output.
const EXIT_UNVERIFIED: u8 = 3;
/// Exit code after Ctrl+C (128 + SIGINT, as shells report it).
const EXIT_CANCELED: u8 = 130;

/// Set by the Ctrl+C handler; the render polls it once per frame.
static CANCEL: AtomicBool = AtomicBool::new(false);

fn main() -> ExitCode {
    let cli = Cli::parse();
    init_logging(&cli.global);
    if let Err(error) = ctrlc::set_handler(|| CANCEL.store(true, Ordering::Relaxed)) {
        tracing::warn!(%error, "Ctrl+C will not cancel cleanly");
    }
    match cli.command {
        Command::Probe(ref args) => match probe::probe(args, cli.global.json) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                write_error(&error, cli.global.json);
                ExitCode::FAILURE
            }
        },
        Command::Render(ref args) => {
            let mut events = Events::new(blurrd_core::JobId(Uuid::now_v7()), cli.global.json);
            match render::render(args, &cli.global, &mut events, &CANCEL) {
                Ok(Outcome::Rendered { unverified: false }) => ExitCode::SUCCESS,
                Ok(Outcome::Rendered { unverified: true }) => ExitCode::from(EXIT_UNVERIFIED),
                Ok(Outcome::Canceled) => ExitCode::from(EXIT_CANCELED),
                Err(error) => {
                    events.failed(&error);
                    ExitCode::FAILURE
                }
            }
        }
    }
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
            &blurrd_core::AppError::new(
                blurrd_core::ErrorKind::Internal,
                "Logging could not be set up.",
            ),
            global.json,
        );
    }
}
