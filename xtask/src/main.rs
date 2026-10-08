//! Developer tasks for the blurrd repository.
//!
//! Run `cargo xtask <task>` from anywhere inside the repository (the alias is
//! defined in `.cargo/config.toml`); `cargo xtask help` lists the tasks.
//! Tasks are added by the milestones that need them: generating TypeScript
//! bindings and JSON schemas, fetching the pinned FFmpeg build and staging
//! its DLLs, fetching the fonts of the synthetic test fixtures and checking
//! the local test recordings so far.

mod codegen;
mod ffmpeg;
mod fonts;
mod media;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};

/// Developer tasks for the blurrd repository.
#[derive(Debug, Parser)]
#[command(name = "cargo xtask", bin_name = "cargo xtask")]
struct Cli {
    #[command(subcommand)]
    task: Task,
}

#[derive(Debug, Subcommand)]
enum Task {
    /// Regenerate the TypeScript bindings of the IPC types (ts-rs).
    ///
    /// Writes into `TS_RS_EXPORT_DIR` (set in `.cargo/config.toml`) and
    /// removes bindings of types that no longer exist.
    Bindings,
    /// Regenerate the JSON Schemas of the persisted documents in
    /// `docs/schema/`.
    Schema,
    /// Install the FFmpeg build pinned in `third_party/ffmpeg.toml` into
    /// `third_party/ffmpeg/` and stage its DLLs.
    ///
    /// Downloads the release asset (with `GH_TOKEN`, `GITHUB_TOKEN` or the
    /// GitHub CLI login while the repository is private), verifies its
    /// SHA-256 and unpacks it. Does nothing but staging when the pinned
    /// build is already installed.
    FetchFfmpeg {
        /// Use this local copy of the pinned archive instead of downloading
        /// it; it must still match the pinned SHA-256.
        #[arg(long, value_name = "ZIP")]
        archive: Option<PathBuf>,
    },
    /// Copy the FFmpeg DLLs next to the binaries in `target/` (every build
    /// profile, including the test binaries in `deps/`).
    StageDlls,
    /// Download the fonts pinned in `tests/fonts/fonts.toml` (names in
    /// other scripts for the synthetic HUD fixtures) into
    /// `tests/fonts/noto/`, verifying each SHA-256.
    FetchFonts,
    /// Work with the local test recordings listed in
    /// `tests/media/manifest.toml`.
    Media {
        #[command(subcommand)]
        action: MediaAction,
    },
}

#[derive(Debug, Subcommand)]
enum MediaAction {
    /// Check that every recording is present with its pinned SHA-256.
    Verify {
        /// The folder with the recordings (default: `BLURRD_TEST_MEDIA`).
        #[arg(long, value_name = "DIR")]
        media: Option<PathBuf>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let root = workspace_root();
    let result = match cli.task {
        Task::Bindings => codegen::bindings(&codegen::bindings_dir(&root)).map(report_written),
        Task::Schema => codegen::schemas(&root.join("docs").join("schema")).map(report_written),
        Task::FetchFfmpeg { archive } => {
            let source = archive.map_or(ffmpeg::Source::Release, ffmpeg::Source::Archive);
            ffmpeg::fetch(&root, &source)
        }
        Task::StageDlls => ffmpeg::stage_dlls(&root),
        Task::FetchFonts => fonts::fetch(&root),
        Task::Media {
            action: MediaAction::Verify { media },
        } => media::verify(media),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn report_written(count: usize) {
    eprintln!("wrote {count} files");
}

/// The repository root (the parent of the `xtask` package).
fn workspace_root() -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.parent().unwrap_or(manifest_dir).to_path_buf()
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::Cli;

    #[test]
    fn cli_is_consistent() {
        Cli::command().debug_assert();
    }
}
