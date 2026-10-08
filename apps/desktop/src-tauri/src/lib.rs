//! The blurrd desktop app: a thin Tauri shell around the blurrd engine.
//!
//! The React frontend in `apps/desktop/src` talks to this crate over Tauri
//! IPC. The crate owns the window, the plugins and, in later milestones, the
//! commands that start engine jobs.

mod logging;

use tauri::{AppHandle, Manager};
use tauri_plugin_store::StoreBuilder;

/// Label of the main window in `tauri.conf.json`.
const MAIN_WINDOW: &str = "main";

/// The settings file (schema `blurrd-settings/1`), relative to the app
/// data directory (`%APPDATA%\io.github.konbuku.blurrd`). The frontend
/// opens it with `getStore` under the same name.
const SETTINGS_FILE: &str = "settings.json";

/// Builds the app from `tauri.conf.json` and runs it until the last window
/// closes.
///
/// Plugins, in registration order: single instance (a second start focuses
/// the running window instead), log, window state (size, position and
/// maximised state are restored on start and saved on exit), and store
/// (holds the settings file, which the setup hook opens).
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
        .plugin(tauri_plugin_store::Builder::default().build())
        .setup(|app| {
            tauri_plugin_log::log::info!("blurrd {} started", env!("CARGO_PKG_VERSION"));
            open_settings(app.handle());
            Ok(())
        })
        .run(tauri::generate_context!())
}

/// Opens the settings file in the store plugin, so the frontend can reach
/// it with `getStore`; the frontend may not open store files itself. A
/// missing file is created on the first save. An unreadable file is
/// logged and left untouched: the frontend then runs on defaults and does
/// not save.
fn open_settings(app: &AppHandle) {
    if let Err(error) = StoreBuilder::new(app, SETTINGS_FILE)
        .disable_auto_save()
        .build()
    {
        tauri_plugin_log::log::warn!("could not open {SETTINGS_FILE}: {error}");
    }
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
