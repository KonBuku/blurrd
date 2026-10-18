//! The blurrd desktop app executable.

// Release builds are GUI applications without a console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::process::ExitCode;

fn main() -> ExitCode {
    // Before anything loads a DLL at run time (ONNX Runtime, drivers).
    blurrd_video::sys::dll::restrict_dll_search();
    match blurrd_app::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("blurrd failed to start: {error}");
            ExitCode::FAILURE
        }
    }
}
