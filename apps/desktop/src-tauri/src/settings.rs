//! The backend's sections of the settings file (architecture plan 6.3):
//! processing, the defaults for new files and privacy.
//!
//! The frontend writes `appearance` and `general` through the store plugin
//! itself; the sections here are read and written only through commands,
//! so every value is checked by the Rust types before it reaches the file.
//! Both sides use the same store (the setup hook opens it), and each write
//! sets the schema header too. A file of another schema major (written by
//! a newer blurrd) is read but never written. A section that is missing or
//! invalid reads as its default, like a missing field.
//!
//! Processing takes effect when the engine is created at start; the log
//! level applies at once. The output folder of the defaults is a folder
//! the user chose; `save_defaults` records it in a file of its own that
//! the page cannot write ([`CHOSEN_FOLDER_FILE`]), and only a recorded
//! folder that is still the stored one is allowed again at start (the
//! page can write the settings file, so its folder alone proves
//! nothing).

use std::path::{Path, PathBuf};

use blurrd_core::{
    AppError, Defaults, Document, ErrorKind, LogLevel, OutputFolder, Privacy, Processing, Settings,
    StoredSettings, Theme,
};
use serde::de::DeserializeOwned;
use serde_json::Value;
use tauri::{AppHandle, Runtime};
use tauri_plugin_log::log::LevelFilter;
use tauri_plugin_store::StoreExt;

use crate::output;
use crate::paths::UserPaths;

/// The settings file (schema `blurrd-settings/1`), relative to the app
/// data directory (`%APPDATA%\io.github.konbuku.blurrd`). The frontend
/// opens it with `getStore` under the same name.
pub(crate) const SETTINGS_FILE: &str = "settings.json";

/// The record of the output folder `save_defaults` accepted, relative to
/// the app config directory. Only the backend writes it: the page may
/// only reach the store files the backend opens (the settings file).
pub(crate) const CHOSEN_FOLDER_FILE: &str = "chosen-output-folder.json";

/// Keys of the sections the backend owns.
const PROCESSING: &str = "processing";
const DEFAULTS: &str = "defaults";
const PRIVACY: &str = "privacy";

/// The settings the app needs before the window and the engine exist.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Startup {
    /// The stored theme, painted before the page loads.
    pub(crate) theme: Theme,
    /// Processing, for the engine.
    pub(crate) processing: Processing,
    /// Privacy (the log level).
    pub(crate) privacy: Privacy,
    /// The output folder of the defaults, when one is chosen.
    pub(crate) output_folder: Option<PathBuf>,
}

impl Default for Startup {
    fn default() -> Self {
        Self::read(|_| None)
    }
}

impl Startup {
    /// Reads the startup settings from a file whose top-level keys `get`
    /// answers.
    pub(crate) fn read(get: impl Fn(&str) -> Option<Value>) -> Self {
        let theme = get("appearance")
            .as_ref()
            .and_then(|appearance| appearance.get("theme"))
            .and_then(|theme| serde_json::from_value(theme.clone()).ok())
            .unwrap_or_default();
        let defaults = read_defaults(get(DEFAULTS).as_ref());
        let output_folder = match defaults.export.output.folder {
            OutputFolder::Folder(folder) => Some(folder),
            _ => None,
        };
        Self {
            theme,
            processing: read_processing(get(PROCESSING).as_ref()),
            privacy: section(get(PRIVACY).as_ref()),
            output_folder,
        }
    }
}

/// A section from its JSON value: the default when it is missing or does
/// not match the type.
fn section<T: DeserializeOwned + Default>(value: Option<&Value>) -> T {
    value
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .unwrap_or_default()
}

/// The processing section; out-of-range values read as the default.
fn read_processing(value: Option<&Value>) -> Processing {
    let processing: Processing = section(value);
    if processing.problem().is_some() {
        return Processing::default();
    }
    processing
}

/// The defaults section; invalid values read as the default.
fn read_defaults(value: Option<&Value>) -> Defaults {
    let defaults: Defaults = section(value);
    if defaults.problem().is_some() || !output::is_valid_template(&defaults.export.output.template)
    {
        return Defaults::default();
    }
    defaults
}

/// Looks up a top-level key of the settings file.
type Lookup<'a> = dyn Fn(&str) -> Option<Value> + 'a;

/// Whether a file with this `schema` value may be written: no header yet,
/// or the schema this build writes.
fn is_writable(schema: Option<&Value>) -> bool {
    schema.is_none_or(|schema| schema.as_str() == Some(Settings::schema_id().as_str()))
}

/// What `get_settings` answers for a file whose keys `get` answers (`None`
/// when the file could not be opened), with the processing the running
/// engine uses.
pub(crate) fn stored(get: Option<&Lookup<'_>>, active: &Processing) -> StoredSettings {
    let Some(get) = get else {
        return StoredSettings {
            processing: Processing::default(),
            active_processing: active.clone(),
            defaults: Defaults::default(),
            privacy: Privacy::default(),
            writable: false,
        };
    };
    StoredSettings {
        processing: read_processing(get(PROCESSING).as_ref()),
        active_processing: active.clone(),
        defaults: read_defaults(get(DEFAULTS).as_ref()),
        privacy: section(get(PRIVACY).as_ref()),
        writable: is_writable(get("schema").as_ref()),
    }
}

/// Checks processing settings before they are saved.
///
/// # Errors
///
/// [`ErrorKind::InvalidProject`] for out-of-range values.
pub(crate) fn checked_processing(processing: Processing) -> Result<Processing, AppError> {
    match processing.problem() {
        Some(problem) => Err(invalid(problem)),
        None => Ok(processing),
    }
}

/// Checks defaults before they are saved: valid blur, export and
/// verification values, a file name template that names a file, and an
/// output folder the user chose (stored in its canonical form).
///
/// # Errors
///
/// [`ErrorKind::InvalidProject`] for invalid values, the path errors of
/// [`UserPaths::existing`] for a folder that is missing or not chosen.
pub(crate) fn checked_defaults(
    paths: &UserPaths,
    mut defaults: Defaults,
) -> Result<Defaults, AppError> {
    if let Some(problem) = defaults.problem() {
        return Err(invalid(problem));
    }
    if !output::is_valid_template(&defaults.export.output.template) {
        return Err(invalid(format!(
            "the file name template `{}` does not name a valid file",
            defaults.export.output.template
        )));
    }
    if let OutputFolder::Folder(folder) = &defaults.export.output.folder {
        let folder = paths.existing(folder)?;
        if !folder.is_dir() {
            return Err(invalid(format!("{} is not a folder", folder.display())));
        }
        defaults.export.output.folder = OutputFolder::Folder(folder);
    }
    Ok(defaults)
}

/// Checks defaults like [`checked_defaults`] and records their output
/// folder (or that there is none) in `record`, the backend's own file
/// ([`CHOSEN_FOLDER_FILE`]), before the caller saves them.
///
/// # Errors
///
/// See [`checked_defaults`]; [`ErrorKind::Io`] when the record cannot be
/// written.
pub(crate) fn accepted_defaults(
    paths: &UserPaths,
    record: &Path,
    defaults: Defaults,
) -> Result<Defaults, AppError> {
    let defaults = checked_defaults(paths, defaults)?;
    let folder = match &defaults.export.output.folder {
        OutputFolder::Folder(folder) => Some(folder.as_path()),
        _ => None,
    };
    record_chosen_folder(record, folder)?;
    Ok(defaults)
}

/// Writes the output folder the user chose to `record` (removes the
/// record when there is none).
fn record_chosen_folder(record: &Path, folder: Option<&Path>) -> Result<(), AppError> {
    let not_saved = |error: std::io::Error| {
        AppError::new(ErrorKind::Io, "The settings could not be saved.")
            .with_detail(format!("{}: {error}", record.display()))
    };
    match folder {
        Some(folder) => {
            if let Some(parent) = record.parent() {
                std::fs::create_dir_all(parent).map_err(not_saved)?;
            }
            let text = serde_json::json!({ "folder": folder }).to_string();
            std::fs::write(record, text).map_err(not_saved)
        }
        None => match std::fs::remove_file(record) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(not_saved(error)),
            _ => Ok(()),
        },
    }
}

/// The output folder to allow again at start: the folder recorded in
/// `record` by [`accepted_defaults`], when it is also the `stored` output
/// folder of the settings file. A folder only the settings file names is
/// not allowed (the page can write that file).
pub(crate) fn restorable_folder(record: &Path, stored: Option<&Path>) -> Option<PathBuf> {
    let stored = stored?;
    let text = std::fs::read_to_string(record).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    let recorded = PathBuf::from(value.get("folder")?.as_str()?);
    (recorded == stored).then_some(recorded)
}

fn invalid(problem: impl Into<String>) -> AppError {
    AppError::new(ErrorKind::InvalidProject, "The settings are invalid.").with_detail(problem)
}

/// What `get_settings` answers.
pub(crate) fn get<R: Runtime>(app: &AppHandle<R>, active: &Processing) -> StoredSettings {
    match app.get_store(SETTINGS_FILE) {
        Some(store) => stored(Some(&|key: &str| store.get(key)), active),
        None => stored(None, active),
    }
}

/// Writes one section of the backend with the schema header.
///
/// # Errors
///
/// [`ErrorKind::Io`] when the file is not open, belongs to another schema
/// major or cannot be saved.
pub(crate) fn save<R: Runtime>(
    app: &AppHandle<R>,
    key: &str,
    value: Value,
) -> Result<(), AppError> {
    let not_saved = |detail: String| {
        AppError::new(ErrorKind::Io, "The settings could not be saved.").with_detail(detail)
    };
    let store = app
        .get_store(SETTINGS_FILE)
        .ok_or_else(|| not_saved(format!("{SETTINGS_FILE} is not open")))?;
    if !is_writable(store.get("schema").as_ref()) {
        return Err(not_saved(format!(
            "{SETTINGS_FILE} was written by another version of blurrd"
        )));
    }
    store.set("schema", Settings::schema_id());
    store.set("version", Settings::REVISION);
    store.set(key, value);
    store.save().map_err(|error| not_saved(error.to_string()))
}

/// Saves the processing section.
///
/// # Errors
///
/// See [`save`].
pub(crate) fn save_processing<R: Runtime>(
    app: &AppHandle<R>,
    processing: &Processing,
) -> Result<(), AppError> {
    save(app, PROCESSING, to_value(processing)?)
}

/// Saves the defaults section.
///
/// # Errors
///
/// See [`save`].
pub(crate) fn save_defaults<R: Runtime>(
    app: &AppHandle<R>,
    defaults: &Defaults,
) -> Result<(), AppError> {
    save(app, DEFAULTS, to_value(defaults)?)
}

/// Saves the privacy section.
///
/// # Errors
///
/// See [`save`].
pub(crate) fn save_privacy<R: Runtime>(
    app: &AppHandle<R>,
    privacy: Privacy,
) -> Result<(), AppError> {
    save(app, PRIVACY, to_value(&privacy)?)
}

fn to_value(value: &impl serde::Serialize) -> Result<Value, AppError> {
    serde_json::to_value(value).map_err(|error| {
        AppError::new(ErrorKind::Internal, "The settings could not be saved.")
            .with_detail(error.to_string())
    })
}

/// The log filter of a log level.
pub(crate) fn level_filter(level: LogLevel) -> LevelFilter {
    match level {
        LogLevel::Error => LevelFilter::Error,
        LogLevel::Warn => LevelFilter::Warn,
        LogLevel::Debug => LevelFilter::Debug,
        LogLevel::Trace => LevelFilter::Trace,
        LogLevel::Info => LevelFilter::Info,
    }
}

/// Applies a log level to every record from now on (the log plugin
/// filters below it: see [`crate::logging`]).
pub(crate) fn apply_log_level(level: LogLevel) {
    tauri_plugin_log::log::set_max_level(level_filter(level));
}

/// The cached analyses in `cache` (`<cache>\projects\<id>`): how many and
/// how many bytes they take.
pub(crate) fn project_usage(cache: &Path) -> (u32, u64) {
    let Ok(entries) = std::fs::read_dir(cache.join("projects")) else {
        return (0, 0);
    };
    entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .fold((0, 0), |(count, bytes), entry| {
            (
                count.saturating_add(1),
                bytes.saturating_add(folder_bytes(&entry.path())),
            )
        })
}

/// Bytes of the files below `folder` (links are not followed).
fn folder_bytes(folder: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| match entry.file_type() {
            Ok(kind) if kind.is_dir() => folder_bytes(&entry.path()),
            Ok(kind) if kind.is_file() => entry.metadata().map_or(0, |meta| meta.len()),
            _ => 0,
        })
        .sum()
}

/// Deletes the cached analyses in `cache` except those of `keep` (the
/// projects open in this session, whose files a job may be using).
/// Answers how many folders could not be deleted.
pub(crate) fn clear_projects(cache: &Path, keep: &[String]) -> usize {
    let Ok(entries) = std::fs::read_dir(cache.join("projects")) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .filter(|entry| !keep.iter().any(|id| entry.file_name() == id.as_str()))
        .filter(|entry| {
            std::fs::remove_dir_all(entry.path())
                .inspect_err(|error| {
                    tauri_plugin_log::log::warn!("could not delete a cached analysis: {error}");
                })
                .is_err()
        })
        .count()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use blurrd_core::{
        Defaults, ExportSettings, HwDecodeSetting, LogLevel, OutputFolder, Privacy, Processing,
        Theme,
    };
    use serde_json::{Value, json};

    use super::{
        Startup, accepted_defaults, checked_defaults, checked_processing, clear_projects,
        is_writable, level_filter, project_usage, restorable_folder, stored,
    };
    use crate::paths::tests::paths_below;

    fn file(value: Value) -> impl Fn(&str) -> Option<Value> {
        let map: BTreeMap<String, Value> = serde_json::from_value(value).unwrap();
        move |key| map.get(key).cloned()
    }

    #[test]
    fn missing_sections_read_as_defaults() {
        let read = stored(Some(&file(json!({}))), &Processing::default());
        assert_eq!(read.processing, Processing::default());
        assert_eq!(read.defaults, Defaults::default());
        assert_eq!(read.privacy, Privacy::default());
        assert!(read.writable);
        assert_eq!(Startup::default().theme, Theme::Dark);
    }

    #[test]
    fn stored_sections_are_read() {
        // The frontend always writes whole sections.
        let mut export = serde_json::to_value(ExportSettings::default()).unwrap();
        export["container"] = json!("Mkv");
        export["output"] = json!({ "folder": { "Folder": "D:\\out" }, "template": "{name}_x",
                                   "on_conflict": "Skip" });
        let get = file(json!({
            "schema": "blurrd-settings/1", "version": 1,
            "appearance": { "theme": "Light" },
            "processing": { "hw_decode": "Software" },
            "privacy": { "log_level": "Debug" },
            "defaults": { "export": export }
        }));
        let read = stored(Some(&get), &Processing::default());
        assert_eq!(read.processing.hw_decode, HwDecodeSetting::Software);
        assert!(!read.processing.uses_gpu());
        assert_eq!(read.privacy.log_level, LogLevel::Debug);
        assert_eq!(read.defaults.export.output.template, "{name}_x");
        let startup = Startup::read(get);
        assert_eq!(startup.theme, Theme::Light);
        assert_eq!(startup.output_folder, Some("D:\\out".into()));
        assert_eq!(startup.privacy.log_level, LogLevel::Debug);
    }

    #[test]
    fn invalid_sections_read_as_defaults() {
        let get = file(json!({
            "processing": { "parallel_jobs": 9 },
            "privacy": { "log_level": "Loud" },
            "defaults": { "export": { "preset": 42 } }
        }));
        let read = stored(Some(&get), &Processing::default());
        assert_eq!(read.processing, Processing::default());
        assert_eq!(read.privacy, Privacy::default());
        assert_eq!(read.defaults, Defaults::default());
        let bad_template = file(json!({
            "defaults": { "export": { "output": { "template": "a:b" } } }
        }));
        assert_eq!(
            stored(Some(&bad_template), &Processing::default()).defaults,
            Defaults::default()
        );
    }

    #[test]
    fn files_of_another_major_are_read_only() {
        assert!(is_writable(None));
        assert!(is_writable(Some(&json!("blurrd-settings/1"))));
        assert!(!is_writable(Some(&json!("blurrd-settings/2"))));
        let read = stored(
            Some(&file(json!({ "schema": "blurrd-settings/2" }))),
            &Processing::default(),
        );
        assert!(!read.writable);
        assert!(!stored(None, &Processing::default()).writable);
    }

    #[test]
    fn the_active_processing_is_reported_beside_the_saved_one() {
        let active = Processing {
            hw_decode: HwDecodeSetting::Software,
            ..Processing::default()
        };
        let read = stored(Some(&file(json!({}))), &active);
        assert_eq!(read.active_processing, active);
        assert_eq!(read.processing, Processing::default());
    }

    #[test]
    fn processing_is_checked() {
        assert!(checked_processing(Processing::default()).is_ok());
        let bad = Processing {
            parallel_jobs: 3,
            ..Processing::default()
        };
        assert!(checked_processing(bad).is_err());
    }

    #[test]
    fn defaults_need_a_valid_template_and_a_chosen_folder() {
        let dir = std::env::temp_dir().join(format!("blurrd-settings-{}", std::process::id()));
        let chosen = dir.join("chosen");
        let other = dir.join("other");
        std::fs::create_dir_all(&chosen).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        let paths = paths_below(&chosen);

        assert_eq!(
            checked_defaults(&paths, Defaults::default()).unwrap(),
            Defaults::default()
        );
        let mut defaults = Defaults::default();
        defaults.export.output.template = "{name}?".to_owned();
        assert!(checked_defaults(&paths, defaults).is_err());

        let mut defaults = Defaults::default();
        defaults.export.output.folder = OutputFolder::Folder(other.clone());
        assert!(checked_defaults(&paths, defaults).is_err());

        let mut defaults = Defaults::default();
        defaults.export.output.folder = OutputFolder::Folder(chosen.clone());
        let checked = checked_defaults(&paths, defaults).unwrap();
        assert_eq!(
            checked.export.output.folder,
            OutputFolder::Folder(crate::paths::simplified(&chosen.canonicalize().unwrap()))
        );

        let mut defaults = Defaults::default();
        defaults.export.preset = 0;
        assert!(checked_defaults(&paths, defaults).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn only_a_folder_saved_by_the_backend_is_allowed_again() {
        let dir = std::env::temp_dir().join(format!("blurrd-record-{}", std::process::id()));
        let chosen = dir.join("chosen");
        let planted = dir.join("planted");
        std::fs::create_dir_all(&chosen).unwrap();
        std::fs::create_dir_all(&planted).unwrap();
        let record = dir.join("config").join("chosen-output-folder.json");
        let paths = paths_below(&chosen);
        let stored_folder = |defaults: &Defaults| {
            let get = file(json!({ "defaults": serde_json::to_value(defaults).unwrap() }));
            Startup::read(get).output_folder
        };

        // A folder the page planted in the settings file: no record.
        let mut planted_defaults = Defaults::default();
        planted_defaults.export.output.folder = OutputFolder::Folder(planted.clone());
        let stored = stored_folder(&planted_defaults);
        assert_eq!(stored, Some(planted.clone()));
        assert_eq!(restorable_folder(&record, stored.as_deref()), None);

        // A folder saved through save_defaults is restored.
        let mut defaults = Defaults::default();
        defaults.export.output.folder = OutputFolder::Folder(chosen.clone());
        let saved = accepted_defaults(&paths, &record, defaults).unwrap();
        let canonical = crate::paths::simplified(&chosen.canonicalize().unwrap());
        assert_eq!(
            restorable_folder(&record, stored_folder(&saved).as_deref()),
            Some(canonical)
        );

        // Planted over a saved one: the record does not match.
        assert_eq!(restorable_folder(&record, stored.as_deref()), None);

        // A folder the user did not choose is refused and not recorded.
        assert!(accepted_defaults(&paths, &record, planted_defaults).is_err());
        assert!(restorable_folder(&record, stored_folder(&saved).as_deref()).is_some());

        // Back to "next to the recording": the record goes.
        accepted_defaults(&paths, &record, Defaults::default()).unwrap();
        assert!(!record.exists());
        assert_eq!(
            restorable_folder(&record, stored_folder(&saved).as_deref()),
            None
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn log_levels_map_to_filters() {
        use tauri_plugin_log::log::LevelFilter;
        assert_eq!(level_filter(LogLevel::Error), LevelFilter::Error);
        assert_eq!(level_filter(LogLevel::Info), LevelFilter::Info);
        assert_eq!(level_filter(LogLevel::Debug), LevelFilter::Debug);
    }

    #[test]
    fn clearing_the_cache_keeps_open_projects() {
        let cache = std::env::temp_dir().join(format!("blurrd-cache-{}", std::process::id()));
        for id in ["aaa", "bbb", "ccc"] {
            let dir = cache.join("projects").join(id);
            std::fs::create_dir_all(dir.join("thumbs")).unwrap();
            std::fs::write(dir.join("project.json"), b"{}").unwrap();
            std::fs::write(dir.join("thumbs").join("00000.jpg"), b"1234").unwrap();
        }
        std::fs::write(cache.join("session.json"), b"[]").unwrap();
        assert_eq!(project_usage(&cache), (3, 18));
        assert_eq!(clear_projects(&cache, &["bbb".to_owned()]), 0);
        assert_eq!(project_usage(&cache), (1, 6));
        assert!(cache.join("projects").join("bbb").is_dir());
        assert!(cache.join("session.json").is_file());
        std::fs::remove_dir_all(&cache).unwrap();
        assert_eq!(project_usage(&cache), (0, 0));
    }
}
