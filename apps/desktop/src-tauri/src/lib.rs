//! The blurrd desktop app: a thin Tauri shell around the blurrd engine.
//!
//! The React frontend in `apps/desktop/src` talks to this crate over Tauri
//! IPC. The crate owns the window, the plugins and, in later milestones, the
//! commands that start engine jobs.

mod logging;

use tauri::{AppHandle, Manager};

/// Label of the main window in `tauri.conf.json`.
const MAIN_WINDOW: &str = "main";

/// Builds the app from `tauri.conf.json` and runs it until the last window
/// closes.
///
/// Plugins, in registration order: single instance (a second start focuses
/// the running window instead), log, and window state (size, position and
/// maximised state are restored on start and saved on exit).
///
/// # Errors
///
/// Returns the Tauri error when the app cannot be built, for example when
/// the WebView2 runtime is missing.
pub fn run() -> Result<(), tauri::Error> {
    tauri::Builder::default()
        // Must be registered first so that a second instance exits before
        // any other plugin touches the window or the log file.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            focus_main_window(app);
        }))
        .plugin(logging::plugin())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .setup(|_app| {
            tauri_plugin_log::log::info!("blurrd {} started", env!("CARGO_PKG_VERSION"));
            Ok(())
        })
        .run(tauri::generate_context!())
}

/// Brings the main window to the front, restoring it when it is minimised.
fn focus_main_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW) else {
        return;
    };
    let result = window
        .unminimize()
        .and_then(|()| window.show())
        .and_then(|()| window.set_focus());
    if let Err(error) = result {
        tauri_plugin_log::log::warn!("could not focus the main window: {error}");
    }
}
