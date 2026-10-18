//! Generates the Tauri context (configuration, icons, capabilities) and the
//! Windows resources of the app executable, and declares the app's own
//! commands so that capabilities must grant each of them.

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

fn main() {
    let manifest = tauri_build::AppManifest::new().commands(COMMANDS);
    if let Err(error) =
        tauri_build::try_build(tauri_build::Attributes::new().app_manifest(manifest))
    {
        panic!("tauri build failed: {error:#}");
    }
}
