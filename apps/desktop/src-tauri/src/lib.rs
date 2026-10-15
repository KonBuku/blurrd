//! The blurrd desktop app: a thin Tauri shell around the blurrd engine.
//!
//! The React frontend in `apps/desktop/src` talks to this crate over Tauri
//! IPC. The crate owns the window and the plugins, and exposes the
//! engine through its commands (module `commands`): probing and opening
//! videos, reading and editing projects, presets and the saved queue, and
//! jobs that report over per-job channels (module `jobs`). The
//! `blurrd://` scheme serves thumbnails, Exact patches and leak crops
//! (module `scheme`).

mod capabilities;
mod commands;
mod exact;
mod jobs;
mod logging;
mod media;
mod media_server;
mod output;
mod paths;
mod presets;
mod projects;
mod scheme;
mod session;
mod state;
mod tasks;

use std::path::Path;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, RunEvent};
use tauri_plugin_store::StoreBuilder;

use crate::paths::UserPaths;
use crate::state::AppState;

/// Label of the main window in `tauri.conf.json`.
const MAIN_WINDOW: &str = "main";

/// The settings file (schema `blurrd-settings/1`), relative to the app
/// data directory (`%APPDATA%\io.github.konbuku.blurrd`). The frontend
/// opens it with `getStore` under the same name.
const SETTINGS_FILE: &str = "settings.json";

/// Event that hands the files of a second app start to the frontend,
/// which adds them to the queue.
const SECOND_INSTANCE_EVENT: &str = "second-instance";

/// Payload of [`SECOND_INSTANCE_EVENT`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct SecondInstance {
    /// Absolute paths from the command line of the second start.
    paths: Vec<String>,
}

/// Builds the app from `tauri.conf.json` and runs it until the last window
/// closes.
///
/// Plugins, in registration order: single instance (a second start focuses
/// the running window instead and hands its file arguments to the
/// frontend), log, window state (size, position and maximised state are
/// restored on start and saved on exit), store (holds the settings file,
/// which the setup hook opens), dialog (open and save dialogs),
/// notification (a system notification when a job ends while the window
/// is in the background) and opener (shows an exported file in Explorer).
///
/// # Errors
///
/// Returns the Tauri error when the app cannot be built, for example when
/// the WebView2 runtime is missing.
pub fn run() -> Result<(), tauri::Error> {
    tauri::Builder::default()
        // Must be registered first so that a second instance exits before
        // any other plugin touches the window or the log file.
        .plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
            focus_main_window(app);
            forward_files(app, &argv, &cwd);
        }))
        .plugin(logging::plugin())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            tauri_plugin_log::log::info!("blurrd {} started", env!("CARGO_PKG_VERSION"));
            open_settings(app.handle());
            let state = app_state(app.handle())?;
            state.probe_capabilities();
            app.manage(state);
            Ok(())
        })
        .register_asynchronous_uri_scheme_protocol(scheme::SCHEME, |ctx, request, responder| {
            let Some(state) = ctx.app_handle().try_state::<AppState>() else {
                // Only before the setup hook ran; the page is not loaded yet.
                responder.respond(scheme::unavailable());
                return;
            };
            let state = state.inner().clone();
            // Decoding and encoding take milliseconds: answer off the
            // webview's thread.
            let spawned = std::thread::Builder::new()
                .name("blurrd-scheme".to_owned())
                .spawn(move || responder.respond(scheme::respond(&state, &request)));
            if let Err(error) = spawned {
                tauri_plugin_log::log::warn!("could not answer a blurrd:// request: {error}");
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::probe_media,
            commands::get_capabilities,
            commands::get_session,
            commands::save_session,
            commands::open_project,
            commands::get_tracks,
            commands::get_pts,
            commands::get_timeline_density,
            commands::get_verify_report,
            commands::set_overrides,
            commands::get_output_preview,
            commands::accept_report,
            commands::start_analysis,
            commands::start_render,
            commands::start_reverify,
            commands::start_run,
            commands::cancel_job,
            commands::pause_queue,
            commands::resume_queue,
            commands::list_presets,
            commands::save_preset,
            commands::delete_preset,
            commands::import_preset,
            commands::export_preset,
        ])
        .build(tauri::generate_context!())?
        .run(|app, event| {
            if let RunEvent::Exit = event
                && let Some(state) = app.try_state::<AppState>()
            {
                // Cancelled jobs remove their partial outputs.
                state.jobs.shutdown();
            }
        });
    Ok(())
}

/// The commands' state: the engine, the project registry, the job queue
/// and the stores, with paths checked against the asset protocol scope
/// that the dialogs and the drop handler fill.
fn app_state(app: &AppHandle) -> Result<AppState, Box<dyn std::error::Error>> {
    let config_dir = app.path().app_config_dir()?;
    let paths = UserPaths::new(Box::new(app.asset_protocol_scope()));
    Ok(AppState::new(&config_dir, paths)?)
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

/// Sends the file arguments of a second start (for example files dropped
/// on the app icon) to the frontend and lets the commands use them.
/// Nothing is sent without file arguments; the backend probes the paths
/// when the frontend adds them to the queue.
fn forward_files(app: &AppHandle, argv: &[String], cwd: &str) {
    let paths = file_arguments(argv, Path::new(cwd));
    if paths.is_empty() {
        return;
    }
    // The user chose these files (dropped on the app icon, "Open with"),
    // like files dropped onto the window: the commands may use them.
    let scope = app.asset_protocol_scope();
    for path in &paths {
        let path = Path::new(path);
        let allowed = if path.is_dir() {
            scope.allow_directory(path, true)
        } else {
            scope.allow_file(path)
        };
        if let Err(error) = allowed {
            tauri_plugin_log::log::warn!("could not allow a file of a second start: {error}");
        }
    }
    if let Err(error) = app.emit(SECOND_INSTANCE_EVENT, SecondInstance { paths }) {
        tauri_plugin_log::log::warn!("could not forward the files of a second start: {error}");
    }
}

/// The file arguments of a command line: every argument after the program
/// name that is not an option (`-x`, `--x`), resolved against `cwd` when
/// relative.
fn file_arguments(argv: &[String], cwd: &Path) -> Vec<String> {
    argv.iter()
        .skip(1)
        .filter(|argument| !argument.is_empty() && !argument.starts_with('-'))
        .map(|argument| cwd.join(argument).to_string_lossy().into_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::file_arguments;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    #[test]
    fn file_arguments_skip_the_program_and_options() {
        let argv = strings(&["blurrd-app.exe", "--flag", "-v", "", r"C:\clips\a.mp4"]);
        assert_eq!(
            file_arguments(&argv, Path::new(r"D:\work")),
            strings(&[r"C:\clips\a.mp4"])
        );
    }

    #[test]
    fn file_arguments_resolve_relative_paths_against_the_working_directory() {
        let argv = strings(&["blurrd-app.exe", "b.mkv"]);
        assert_eq!(
            file_arguments(&argv, Path::new(r"D:\work")),
            strings(&[r"D:\work\b.mkv"])
        );
    }

    #[test]
    fn file_arguments_are_empty_without_arguments() {
        assert_eq!(
            file_arguments(&strings(&["blurrd-app.exe"]), Path::new(".")),
            Vec::<String>::new()
        );
    }
}
