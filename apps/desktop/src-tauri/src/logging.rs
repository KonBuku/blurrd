//! Logging: one JSON object per line in the app's log folder, plus readable
//! lines on stdout in debug builds.
//!
//! Records come from the Rust `log` facade and from the frontend through the
//! log plugin's `log` command. They never contain recognised text (see the
//! privacy rules in `docs/privacy.md` once it exists); only file names are
//! logged at `info`.

use std::time::{SystemTime, UNIX_EPOCH};

use tauri::Runtime;
use tauri::plugin::TauriPlugin;
use tauri_plugin_log::log::{LevelFilter, Record};
use tauri_plugin_log::{RotationStrategy, Target, TargetKind};

/// File name of the log, without the `.log` extension.
const LOG_FILE_NAME: &str = "blurrd";
/// Size at which the log file is rotated.
const MAX_FILE_BYTES: u128 = 5 * 1024 * 1024;
/// Rotated log files kept next to the current one.
const KEPT_FILES: usize = 4;
/// Log targets of blurrd itself: this crate, the engine crates (their
/// tracing events arrive as `log` records named by module) and the
/// frontend's records through the log plugin.
const OWN_TARGETS: &[&str] = &[
    env!("CARGO_CRATE_NAME"),
    "blurrd_engine",
    "blurrd_video",
    "blurrd_ocr",
    "blurrd_detect",
    "blurrd_blur",
    "blurrd_core",
    "webview",
];

/// Builds the log plugin: JSON lines in `%LOCALAPPDATA%\<identifier>\logs`,
/// rotated at 5 MiB and kept four times, and stdout in debug builds.
/// blurrd's own records (the app, the engine crates and the frontend) pass
/// at every level and dependencies at `info` and above; the log level of
/// Settings › Logs & privacy then caps all of them (`log::set_max_level`,
/// see [`crate::settings::apply_log_level`]).
pub(crate) fn plugin<R: Runtime>() -> TauriPlugin<R> {
    let file = Target::new(TargetKind::LogDir {
        file_name: Some(LOG_FILE_NAME.to_owned()),
    })
    .format(|out, message, record| out.finish(format_args!("{}", json_line(record, message))));

    // Each target formats the raw message itself (the builder's default
    // format would prefix a date and the level to it).
    let mut builder = tauri_plugin_log::Builder::new()
        .clear_format()
        .clear_targets()
        .target(file)
        .level(LevelFilter::Info)
        .max_file_size(MAX_FILE_BYTES)
        .rotation_strategy(RotationStrategy::KeepSome(KEPT_FILES));
    for target in OWN_TARGETS {
        builder = builder.level_for(*target, LevelFilter::Trace);
    }
    if cfg!(debug_assertions) {
        // The same records as readable lines.
        builder = builder.target(
            Target::new(TargetKind::Stdout).format(|out, message, record| {
                out.finish(format_args!(
                    "[{} {}] {message}",
                    record.level(),
                    record.target()
                ));
            }),
        );
    }
    builder.build()
}

/// Formats one record as a single-line JSON object with the Unix time in
/// milliseconds, the level, the target and the message.
fn json_line(record: &Record<'_>, message: &std::fmt::Arguments<'_>) -> String {
    let ts_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis());
    serde_json::json!({
        "ts_ms": ts_ms,
        "level": record.level().as_str(),
        "target": record.target(),
        "message": message.to_string(),
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use tauri_plugin_log::log::{Level, Record};

    use super::json_line;

    #[test]
    fn json_lines_escape_the_message_and_stay_on_one_line() {
        let args = format_args!("opened \"clip.mp4\"\nnext");
        let record = Record::builder()
            .level(Level::Info)
            .target("blurrd_app")
            .args(args)
            .build();
        let line = json_line(&record, &args);
        assert!(!line.contains('\n'));
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["level"], "INFO");
        assert_eq!(value["target"], "blurrd_app");
        assert_eq!(value["message"], "opened \"clip.mp4\"\nnext");
        assert!(value["ts_ms"].as_u64().unwrap() > 0);
    }
}
