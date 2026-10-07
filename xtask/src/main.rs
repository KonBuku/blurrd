//! Developer tasks for the blurrd repository.
//!
//! Run `cargo xtask <task>` from anywhere inside the repository (the alias is
//! defined in `.cargo/config.toml`); `cargo xtask help` lists the tasks.
//! Tasks are added by the milestones that need them: generating TypeScript
//! bindings and JSON schemas, fetching the pinned FFmpeg and ONNX Runtime
//! builds, staging runtime DLLs, and more.

mod codegen;

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
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let root = workspace_root();
    let result = match cli.task {
        Task::Bindings => codegen::bindings(&codegen::bindings_dir(&root)),
        Task::Schema => codegen::schemas(&root.join("docs").join("schema")),
    };
    match result {
        Ok(written) => {
            eprintln!("wrote {written} files");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
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
