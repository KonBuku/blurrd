//! The blurrd desktop app: a thin Tauri shell around the blurrd engine.
//!
//! The React frontend in `apps/desktop/src` talks to this crate over Tauri
//! IPC. The crate owns the window, the plugins and, in later milestones, the
//! commands that start engine jobs.

/// Builds the app from `tauri.conf.json` and runs it until the last window
/// closes.
///
/// # Errors
///
/// Returns the Tauri error when the app cannot be built, for example when
/// the WebView2 runtime is missing.
pub fn run() -> Result<(), tauri::Error> {
    tauri::Builder::default().run(tauri::generate_context!())
}
