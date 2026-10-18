//! The blurrd desktop app: a thin Tauri shell around the blurrd engine.
//!
//! The React frontend in `apps/desktop/src` talks to this crate over Tauri
//! IPC. The crate owns the window and the plugins, and exposes the
//! engine through its commands (module `commands`): probing and opening
//! videos, reading and editing projects, presets and the saved queue, and
//! jobs that report over per-job channels (module `jobs`). The
//! `blurrd://` scheme serves thumbnails, poster frames, Exact patches and
//! leak crops (module `scheme`).

mod capabilities;
mod commands;
mod exact;
mod jobs;
mod logging;
mod media;
mod media_server;
mod output;
mod paths;
mod poster;
mod presets;
mod projects;
mod scheme;
mod session;
mod state;
mod tasks;

use std::path::Path;

use blurrd_core::Theme;
use serde::Serialize;
use tauri::webview::Color;
use tauri::{AppHandle, Emitter, Manager, RunEvent, WebviewWindowBuilder};
use tauri_plugin_store::StoreBuilder;

use crate::paths::UserPaths;
use crate::state::AppState;

/// Label of the main window in `tauri.conf.json`.
const MAIN_WINDOW: &str = "main";

/// The settings file (schema `blurrd-settings/1`), relative to the app
/// data directory (`%APPDATA%\io.github.konbuku.blurrd`). The frontend
/// opens it with `getStore` under the same name.
const SETTINGS_FILE: &str = "settings.json";

/// Window shell colours of the dark and the light theme
/// (`--color-background-win-alt` in `apps/desktop/src/styles/globals.css`).
/// The window and its webview paint them until the page has loaded, so a
/// start never flashes white.
const DARK_SHELL: Color = Color(0x2b, 0x2b, 0x2b, 0xff);
const LIGHT_SHELL: Color = Color(0xec, 0xec, 0xee, 0xff);

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
            let theme = open_settings(app.handle());
            create_main_window(app.handle(), theme)?;
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
            // webview's thread, on the scheme's workers.
            let workers = std::sync::Arc::clone(&state.scheme);
            if let Err(pending) = workers.submit(scheme::Pending::new(state, request, responder)) {
                pending.busy();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::probe_media,
            commands::get_capabilities,
            commands::get_session,
            commands::save_session,
            commands::open_project,
            commands::close_project,
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
    let origin = page_origin(app);
    tauri_plugin_log::log::info!("page origin {origin}");
    Ok(AppState::new(&config_dir, paths, &origin)?)
}

/// The origin the main window's page will have, from the configuration:
/// the setup hook runs before the webview has navigated, so the window's
/// live URL is still `about:blank` there.
fn page_origin(app: &AppHandle) -> String {
    let config = app.config();
    let https = config
        .app
        .windows
        .iter()
        .find(|window| window.label == MAIN_WINDOW)
        .is_some_and(|window| window.use_https_scheme);
    let dev_url = if tauri::is_dev() {
        config.build.dev_url.as_ref()
    } else {
        None
    };
    configured_origin(dev_url, https)
}

/// The page origin for a development server at `dev_url`, or for the
/// bundled frontend on the app scheme (served over `https` when `https`).
fn configured_origin(dev_url: Option<&tauri::Url>, https: bool) -> String {
    match dev_url {
        Some(url) => window_origin(url),
        None if cfg!(windows) && https => "https://tauri.localhost".to_owned(),
        None if cfg!(windows) => "http://tauri.localhost".to_owned(),
        None => "tauri://localhost".to_owned(),
    }
}

/// The origin of a page at `url` as the webview reports it, the value
/// Tauri's asset protocol allows: app schemes are served from
/// `http://<scheme>.localhost` on Windows (`<scheme>://localhost`
/// elsewhere), other pages keep their scheme, host and port.
fn window_origin(url: &tauri::Url) -> String {
    match (url.scheme(), url.host_str()) {
        ("data", _) => "null".to_owned(),
        (scheme, _) if scheme != "http" && scheme != "https" => {
            if cfg!(windows) {
                format!("http://{scheme}.localhost")
            } else {
                format!("{scheme}://localhost")
            }
        }
        (scheme, Some(host)) => match url.port() {
            Some(port) => format!("{scheme}://{host}:{port}"),
            None => format!("{scheme}://{host}"),
        },
        _ => "null".to_owned(),
    }
}

/// Opens the settings file in the store plugin, so the frontend can reach
/// it with `getStore`; the frontend may not open store files itself. A
/// missing file is created on the first save. An unreadable file is
/// logged and left untouched: the frontend then runs on defaults and does
/// not save. Answers the stored theme (the default when there is none),
/// which the window paints before the page loads.
fn open_settings(app: &AppHandle) -> Theme {
    match StoreBuilder::new(app, SETTINGS_FILE)
        .disable_auto_save()
        .build()
    {
        Ok(store) => stored_theme(store.get("appearance").as_ref()),
        Err(error) => {
            tauri_plugin_log::log::warn!("could not open {SETTINGS_FILE}: {error}");
            Theme::default()
        }
    }
}

/// The theme of the settings file's `appearance` section; a missing or
/// invalid value is the default, as in the frontend's `parseAppearance`.
fn stored_theme(appearance: Option<&serde_json::Value>) -> Theme {
    appearance
        .and_then(|appearance| appearance.get("theme"))
        .and_then(|theme| serde_json::from_value(theme.clone()).ok())
        .unwrap_or_default()
}

/// Script that runs before the page's own scripts: it hands the window's
/// theme to `public/boot.js` and the appearance code, so the first paint
/// uses the theme of the window background even when the page's cached
/// copy differs from the settings file.
fn boot_theme_script(theme: Theme) -> String {
    let name = match theme {
        Theme::System => "System",
        Theme::Light => "Light",
        Theme::Dark => "Dark",
    };
    format!("window.blurrdBootTheme = \"{name}\";")
}

/// The shell colour of `theme`; `system_dark` tells what "System" follows.
fn shell_color(theme: Theme, system_dark: bool) -> Color {
    match theme {
        Theme::Dark => DARK_SHELL,
        Theme::System if system_dark => DARK_SHELL,
        Theme::Light | Theme::System => LIGHT_SHELL,
    }
}

/// Creates the main window of `tauri.conf.json` (`create: false` there)
/// with the background of `theme`. For "System" the window is created
/// hidden, given the background of the Windows theme it reports and only
/// then shown, so a light Windows never sees a dark frame.
fn create_main_window(app: &AppHandle, theme: Theme) -> tauri::Result<()> {
    let Some(config) = app
        .config()
        .app
        .windows
        .iter()
        .find(|window| window.label == MAIN_WINDOW)
        .cloned()
    else {
        return Err(tauri::Error::WindowNotFound);
    };
    let follows_system = theme == Theme::System;
    let window = WebviewWindowBuilder::from_config(app, &config)?
        .background_color(shell_color(theme, true))
        .initialization_script(boot_theme_script(theme))
        .visible(!follows_system)
        .build()?;
    if follows_system {
        let system_dark = window.theme().map_or(true, |os| os == tauri::Theme::Dark);
        window.set_background_color(Some(shell_color(theme, system_dark)))?;
        window.show()?;
    }
    Ok(())
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
        // The scope matches the text of a path: allow the canonical form,
        // the one the commands check (a `..` in an argument would never
        // match it).
        let Ok(path) = std::fs::canonicalize(path).map(|path| paths::simplified(&path)) else {
            continue;
        };
        let path = path.as_path();
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

    use blurrd_core::Theme;

    use super::{
        DARK_SHELL, LIGHT_SHELL, boot_theme_script, configured_origin, file_arguments, shell_color,
        stored_theme, window_origin,
    };

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
    fn window_origins_match_the_asset_protocol() {
        let origin = |url: &str| window_origin(&tauri::Url::parse(url).unwrap());
        if cfg!(windows) {
            assert_eq!(origin("tauri://localhost"), "http://tauri.localhost");
        }
        assert_eq!(origin("http://tauri.localhost/"), "http://tauri.localhost");
        assert_eq!(
            origin("http://localhost:1420/index.html"),
            "http://localhost:1420"
        );
        assert_eq!(origin("data:text/html,x"), "null");
    }

    #[test]
    fn configured_origins_are_the_page_origins() {
        let dev = tauri::Url::parse("http://localhost:1420").unwrap();
        assert_eq!(
            configured_origin(Some(&dev), false),
            "http://localhost:1420"
        );
        if cfg!(windows) {
            assert_eq!(configured_origin(None, false), "http://tauri.localhost");
            assert_eq!(configured_origin(None, true), "https://tauri.localhost");
        } else {
            assert_eq!(configured_origin(None, false), "tauri://localhost");
        }
    }

    #[test]
    fn a_fresh_install_starts_dark_and_a_stored_choice_wins() {
        assert_eq!(stored_theme(None), Theme::Dark);
        assert_eq!(stored_theme(Some(&serde_json::json!({}))), Theme::Dark);
        assert_eq!(
            stored_theme(Some(&serde_json::json!({ "theme": "Blue" }))),
            Theme::Dark
        );
        assert_eq!(
            stored_theme(Some(&serde_json::json!({ "theme": "Light" }))),
            Theme::Light
        );
        assert_eq!(
            stored_theme(Some(&serde_json::json!({ "theme": "System" }))),
            Theme::System
        );
    }

    #[test]
    fn the_boot_script_names_the_stored_theme() {
        assert_eq!(
            boot_theme_script(Theme::Light),
            r#"window.blurrdBootTheme = "Light";"#
        );
        assert!(boot_theme_script(Theme::System).contains("\"System\""));
        assert!(boot_theme_script(Theme::Dark).contains("\"Dark\""));
    }

    #[test]
    fn the_window_paints_the_shell_colour_of_its_theme() {
        assert_eq!(shell_color(Theme::Dark, false), DARK_SHELL);
        assert_eq!(shell_color(Theme::Light, true), LIGHT_SHELL);
        assert_eq!(shell_color(Theme::System, true), DARK_SHELL);
        assert_eq!(shell_color(Theme::System, false), LIGHT_SHELL);
    }

    #[test]
    fn file_arguments_are_empty_without_arguments() {
        assert_eq!(
            file_arguments(&strings(&["blurrd-app.exe"]), Path::new(".")),
            Vec::<String>::new()
        );
    }
}
