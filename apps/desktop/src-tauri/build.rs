//! Generates the Tauri context (configuration, icons, capabilities) and the
//! Windows resources of the app executable, declares the app's own
//! commands so that capabilities must grant each of them, and makes sure
//! the folder of the installer's bundled files exists.

/// The commands of `src/commands.rs` (architecture plan 9.1). A command
/// missing here would not be checked against the capabilities.
const COMMANDS: &[&str] = &[
    "probe_media",
    "get_capabilities",
    "get_session",
    "save_session",
    "open_project",
    "close_project",
    "get_tracks",
    "get_pts",
    "get_timeline_density",
    "get_verify_report",
    "set_overrides",
    "get_output_preview",
    "accept_report",
    "start_analysis",
    "start_render",
    "start_reverify",
    "start_run",
    "cancel_job",
    "pause_queue",
    "resume_queue",
    "list_presets",
    "save_preset",
    "delete_preset",
    "import_preset",
    "export_preset",
    "get_settings",
    "save_processing",
    "save_defaults",
    "save_privacy",
    "get_cache_info",
    "clear_cache",
    "open_log_dir",
    "refresh_capabilities",
];

/// The folder `cargo xtask stage-bundle` fills with the DLLs, models and
/// licences the installer ships (`bundle.resources` in `tauri.conf.json`).
const INSTALLER_FILES: &str = "../../../target/installer-files";

fn main() {
    // tauri-build copies the bundle resources next to the executable on
    // every build and fails when their folder does not exist. Outside an
    // installer build the folder is empty (nothing staged yet) or holds
    // the files of the last `stage-bundle`, which are the same files a
    // development build loads from `target/` anyway.
    let staged = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(INSTALLER_FILES);
    if let Err(error) = std::fs::create_dir_all(&staged) {
        panic!("could not create {}: {error}", staged.display());
    }
    let manifest = tauri_build::AppManifest::new().commands(COMMANDS);
    if let Err(error) =
        tauri_build::try_build(tauri_build::Attributes::new().app_manifest(manifest))
    {
        panic!("tauri build failed: {error:#}");
    }
}
