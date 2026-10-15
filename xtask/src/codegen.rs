//! Generated files: TypeScript bindings and JSON Schemas of `blurrd-core`.
//!
//! Both outputs are committed. CI regenerates them and fails if the result
//! differs from the repository, so they always match the Rust types.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use blurrd_core::{
    AppError, BlurParams, Capabilities, DensityLanes, DetectorOptions, Document, ExactFrame,
    ExportSettings, FramePts, JobEvent, MediaInfo, OutputPreview, OverrideBinding, OverridesSaved,
    PresetFile, ProbeOutcome, Project, ProjectSummary, RegionSettings, Settings, TrackSet,
    VerifyReport, VerifySettings,
};
use schemars::JsonSchema;
use ts_rs::{Config, TS};

/// Exports one root type and every type it uses.
type ExportFn = fn(&Config) -> Result<(), ts_rs::ExportError>;

/// The bindings directory: `TS_RS_EXPORT_DIR`, or the desktop frontend's
/// `src/bindings` when xtask runs without cargo's environment.
pub(crate) fn bindings_dir(root: &Path) -> PathBuf {
    std::env::var_os("TS_RS_EXPORT_DIR")
        .map_or_else(|| root.join("apps/desktop/src/bindings"), PathBuf::from)
}

/// Writes the TypeScript bindings of every IPC root type and the types they
/// use into `dir`, after removing the previous bindings. Returns the number
/// of files written.
pub(crate) fn bindings(dir: &Path) -> Result<usize> {
    remove_files_with_extension(dir, "ts")?;
    let cfg = Config::from_env()
        .with_out_dir(dir)
        .with_large_int("number");
    let exports: [ExportFn; 22] = [
        Project::export_all,
        Settings::export_all,
        PresetFile::export_all,
        VerifyReport::export_all,
        ExportSettings::export_all,
        BlurParams::export_all,
        RegionSettings::export_all,
        VerifySettings::export_all,
        MediaInfo::export_all,
        Capabilities::export_all,
        JobEvent::export_all,
        DensityLanes::export_all,
        OverrideBinding::export_all,
        AppError::export_all,
        ProbeOutcome::export_all,
        ProjectSummary::export_all,
        TrackSet::export_all,
        FramePts::export_all,
        OverridesSaved::export_all,
        OutputPreview::export_all,
        DetectorOptions::export_all,
        ExactFrame::export_all,
    ];
    for export in exports {
        export(&cfg).with_context(|| format!("exporting bindings to {}", dir.display()))?;
    }
    count_files_with_extension(dir, "ts")
}

/// Writes one JSON Schema per persisted document into `dir`. Returns the
/// number of files written.
pub(crate) fn schemas(dir: &Path) -> Result<usize> {
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    remove_files_with_extension(dir, "json")?;
    write_schema::<Project>(dir)?;
    write_schema::<Settings>(dir)?;
    write_schema::<PresetFile>(dir)?;
    write_schema::<VerifyReport>(dir)?;
    count_files_with_extension(dir, "json")
}

/// Writes `<NAME>.schema.json` for document `D`, titled with its schema id
/// (for example `blurrd-project/1`).
fn write_schema<D: Document + JsonSchema>(dir: &Path) -> Result<()> {
    let mut schema = schemars::schema_for!(D);
    schema.insert("title".to_owned(), <D as Document>::schema_id().into());
    let mut text = serde_json::to_string_pretty(&schema)?;
    text.push('\n');
    let path = dir.join(format!("{}.schema.json", D::NAME));
    fs::write(&path, text).with_context(|| format!("writing {}", path.display()))
}

fn remove_files_with_extension(dir: &Path, extension: &str) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        if path.is_file() && path.extension() == Some(OsStr::new(extension)) {
            fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
        }
    }
    Ok(())
}

fn count_files_with_extension(dir: &Path, extension: &str) -> Result<usize> {
    let mut count = 0;
    for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        if path.is_file() && path.extension() == Some(OsStr::new(extension)) {
            count += 1;
        }
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{bindings, schemas};

    #[test]
    fn generates_bindings_and_schemas() {
        let dir = std::env::temp_dir().join(format!("blurrd-xtask-{}", std::process::id()));
        let ts = dir.join("bindings");
        let json = dir.join("schema");

        assert!(bindings(&ts).unwrap() > 20);
        let project = fs::read_to_string(ts.join("Project.ts")).unwrap();
        assert!(project.contains("export type Project"));
        // Large integers are numbers, not bigint.
        assert!(!project.contains("bigint"));
        // A stale file is removed on the next run.
        fs::write(ts.join("Stale.ts"), "").unwrap();
        bindings(&ts).unwrap();
        assert!(!ts.join("Stale.ts").exists());

        assert_eq!(schemas(&json).unwrap(), 4);
        let schema = fs::read_to_string(json.join("blurrd-project.schema.json")).unwrap();
        assert!(schema.contains("\"title\": \"blurrd-project/1\""));

        fs::remove_dir_all(&dir).unwrap();
    }
}
