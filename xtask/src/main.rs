//! Developer tasks for the blurrd repository.
//!
//! Run `cargo xtask <task>` from anywhere inside the repository (the alias is
//! defined in `.cargo/config.toml`). Tasks are added by the milestones that
//! need them: fetching the pinned FFmpeg and ONNX Runtime builds, staging
//! runtime DLLs, generating TypeScript bindings and JSON schemas, and more.

use std::env;
use std::fmt::Write as _;
use std::process::ExitCode;

/// Tasks known to this version of xtask, with a one-line description each.
const TASKS: &[(&str, &str)] = &[("help", "Print this list of tasks")];

fn main() -> ExitCode {
    let task = env::args().nth(1);
    match task.as_deref() {
        None | Some("help" | "-h" | "--help") => {
            print_help();
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("error: unknown task `{other}`\n");
            eprint!("{}", help_text());
            ExitCode::from(2)
        }
    }
}

#[allow(clippy::print_stdout, reason = "help output belongs on stdout")]
fn print_help() {
    print!("{}", help_text());
}

/// Renders the usage line and the task list.
fn help_text() -> String {
    let width = TASKS.iter().map(|(name, _)| name.len()).max().unwrap_or(0);
    let mut text = String::from("Usage: cargo xtask <task>\n\nTasks:\n");
    for (name, description) in TASKS {
        // Writing into a `String` cannot fail.
        let _ = writeln!(text, "  {name:<width$}  {description}");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::{TASKS, help_text};

    #[test]
    fn help_lists_every_task() {
        let text = help_text();
        for (name, description) in TASKS {
            assert!(
                text.contains(name) && text.contains(description),
                "missing task {name}"
            );
        }
    }
}
