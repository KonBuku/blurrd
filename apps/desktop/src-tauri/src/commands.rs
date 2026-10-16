//! The IPC commands of the desktop app (architecture plan 9.1).
//!
//! Commands are thin: they check paths against the user's choices (see
//! [`crate::paths`]), find the open project and hand the work to the
//! project, preset, session and job modules. Anything that reads files or
//! decodes runs on a blocking thread, so the window's event loop never
//! waits. Every command answers with an [`AppError`] on failure.

#![allow(
    clippy::needless_pass_by_value,
    reason = "Tauri passes command arguments and state by value"
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use blurrd_core::{
    AppError, BlurParams, Capabilities, DensityLanes, DetectorOptions, ErrorKind, ExportSettings,
    FramePts, JobEvent, JobId, OutputFolder, OutputPreview, Override, OverridesSaved, Preset,
    ProbeOutcome, ProjectSummary, RegionSet, RegionSettings, RunFile, TrackSet, VerifyReport,
    VerifySettings,
};
use blurrd_engine::is_verified;
use tauri::State;
use tauri::ipc::Channel;
use uuid::Uuid;

use crate::jobs::EventOut;
use crate::media::{asset_url, video_url};
use crate::output;
use crate::projects::OpenProject;
use crate::session::SessionEntry;
use crate::state::AppState;
use crate::tasks::{self, ExportRequest};

/// Extensions of the files a folder contributes to the queue.
const VIDEO_EXTENSIONS: [&str; 4] = ["mp4", "m4v", "mov", "mkv"];

/// Runs `work` on a blocking thread.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, AppError> + Send + 'static,
) -> Result<T, AppError> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|e| {
            AppError::new(ErrorKind::Internal, "blurrd ran into an internal error.")
                .with_detail(e.to_string())
        })?
}

/// Media facts of the chosen paths; a folder expands to its videos.
#[tauri::command]
pub(crate) async fn probe_media(
    state: State<'_, AppState>,
    paths: Vec<PathBuf>,
) -> Result<Vec<ProbeOutcome>, AppError> {
    let state = state.inner().clone();
    blocking(move || {
        Ok(paths
            .iter()
            .flat_map(|path| probe_path(&state, path))
            .collect())
    })
    .await
}

/// The outcomes of one requested path.
fn probe_path(state: &AppState, path: &Path) -> Vec<ProbeOutcome> {
    let error = |path: &Path, error: AppError| ProbeOutcome::Error {
        path: path.to_path_buf(),
        error,
    };
    let checked = match state.paths.existing(path) {
        Ok(checked) => checked,
        Err(e) => return vec![error(path, e)],
    };
    if checked.is_dir() {
        return match videos_in(&checked) {
            Ok(files) => files.iter().map(|file| probe_file(file)).collect(),
            Err(e) => vec![error(path, e)],
        };
    }
    if !is_video(&checked) {
        return vec![error(
            &checked,
            AppError::new(
                ErrorKind::UnsupportedMedia,
                "This file is not a supported video.",
            ),
        )];
    }
    vec![probe_file(&checked)]
}

fn probe_file(path: &Path) -> ProbeOutcome {
    match blurrd_video::probe(path) {
        Ok(media) => ProbeOutcome::Media {
            path: path.to_path_buf(),
            media: Box::new(media),
        },
        Err(e) => ProbeOutcome::Error {
            path: path.to_path_buf(),
            error: e.into(),
        },
    }
}

/// The videos directly inside `folder`, sorted by name.
fn videos_in(folder: &Path) -> Result<Vec<PathBuf>, AppError> {
    let entries = std::fs::read_dir(folder).map_err(|e| {
        AppError::new(ErrorKind::Io, "The folder could not be read.")
            .with_detail(format!("{}: {e}", folder.display()))
    })?;
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && is_video(path))
        .collect();
    files.sort_by_key(|path| path.file_name().map(std::ffi::OsStr::to_ascii_lowercase));
    Ok(files)
}

fn is_video(path: &Path) -> bool {
    path.extension().is_some_and(|ext| {
        VIDEO_EXTENSIONS
            .iter()
            .any(|known| ext.eq_ignore_ascii_case(known))
    })
}

/// GPUs, decoders, encoders and OCR, as far as the probe got.
#[tauri::command]
pub(crate) fn get_capabilities(state: State<'_, AppState>) -> Capabilities {
    state.capabilities.current()
}

/// The queue saved by the previous run. Its files may be used again.
#[tauri::command]
pub(crate) async fn get_session(state: State<'_, AppState>) -> Result<Vec<SessionEntry>, AppError> {
    let state = state.inner().clone();
    blocking(move || {
        let entries = state.session.load();
        for entry in &entries {
            state.paths.add_file(&entry.path);
        }
        Ok(entries)
    })
    .await
}

/// Replaces the saved queue (files the user did not choose are left out).
#[tauri::command]
pub(crate) async fn save_session(
    state: State<'_, AppState>,
    entries: Vec<SessionEntry>,
) -> Result<(), AppError> {
    let state = state.inner().clone();
    blocking(move || {
        let chosen = entries
            .into_iter()
            .filter_map(|mut entry| {
                entry.path = state.paths.existing(&entry.path).ok()?;
                Some(entry)
            })
            .collect();
        state
            .session
            .save(chosen, |path| state.projects.fingerprint_of(path))
    })
    .await
}

/// Opens the project of a chosen video.
#[tauri::command]
pub(crate) async fn open_project(
    state: State<'_, AppState>,
    path: PathBuf,
) -> Result<ProjectSummary, AppError> {
    let state = state.inner().clone();
    blocking(move || {
        let source = state.paths.existing(&path)?;
        let project = state.projects.open(&source)?;
        Ok(summary(&state, &project))
    })
    .await
}

/// Closes the project of a file the frontend removed: its video is no
/// longer served and the Exact decoder of it is closed. Unknown ids are
/// ignored; a later `open_project` opens the file again.
#[tauri::command]
pub(crate) fn close_project(state: State<'_, AppState>, project_id: String) {
    if state.projects.close(&project_id) {
        if let Some(media) = &state.media {
            media.forget(&project_id);
        }
        state.exact.close(&project_id);
    }
}

/// The summary of `project` with its media URLs.
fn summary(state: &AppState, project: &OpenProject) -> ProjectSummary {
    let proxy = project.dir().proxy();
    let proxy_url = proxy.is_file().then(|| {
        state.paths.add_file(&proxy);
        asset_url(&proxy)
    });
    let media_url = video_url(
        state.media.as_deref(),
        project.id(),
        project.source(),
        project.info().size,
    );
    project.summary(media_url, proxy_url)
}

/// The project `id`.
fn project(state: &AppState, id: &str) -> Result<Arc<OpenProject>, AppError> {
    state.projects.get(id)
}

/// The tracks and edits of a project.
#[tauri::command]
pub(crate) async fn get_tracks(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<TrackSet, AppError> {
    let state = state.inner().clone();
    blocking(move || Ok(project(&state, &project_id)?.tracks())).await
}

/// The presentation timestamps of a project's video.
#[tauri::command]
pub(crate) async fn get_pts(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<FramePts, AppError> {
    let state = state.inner().clone();
    blocking(move || project(&state, &project_id)?.frame_pts()).await
}

/// Per-region box coverage in `buckets` parts of the timeline.
#[tauri::command]
pub(crate) async fn get_timeline_density(
    state: State<'_, AppState>,
    project_id: String,
    buckets: u32,
) -> Result<DensityLanes, AppError> {
    let state = state.inner().clone();
    blocking(move || Ok(project(&state, &project_id)?.density(buckets))).await
}

/// The last verification report, or `null` before the first.
#[tauri::command]
pub(crate) async fn get_verify_report(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<Option<VerifyReport>, AppError> {
    let state = state.inner().clone();
    blocking(move || project(&state, &project_id)?.verify_report()).await
}

/// Replaces the edits of a project (the full list) at `base_rev`.
#[tauri::command]
pub(crate) async fn set_overrides(
    state: State<'_, AppState>,
    project_id: String,
    base_rev: u64,
    overrides: Vec<Override>,
) -> Result<OverridesSaved, AppError> {
    let state = state.inner().clone();
    blocking(move || project(&state, &project_id)?.set_overrides(base_rev, overrides)).await
}

/// Where an export with `settings` would go and what it would cost.
#[tauri::command]
pub(crate) async fn get_output_preview(
    state: State<'_, AppState>,
    project_id: String,
    settings: ExportSettings,
) -> Result<OutputPreview, AppError> {
    let state = state.inner().clone();
    blocking(move || {
        let project = project(&state, &project_id)?;
        let export = checked_export(&state, settings)?;
        let verified = project.project().is_some_and(|p| is_verified(&p));
        output::preview(project.source(), project.info(), &export, verified)
    })
    .await
}

/// `export` with its output folder checked against the user's choices.
fn checked_export(
    state: &AppState,
    mut export: ExportSettings,
) -> Result<ExportSettings, AppError> {
    if let OutputFolder::Folder(folder) = &export.output.folder {
        export.output.folder = OutputFolder::Folder(state.paths.existing(folder)?);
    }
    Ok(export)
}

/// Accepts a report's open leaks and renames the `_UNVERIFIED` output.
#[tauri::command]
pub(crate) async fn accept_report(
    state: State<'_, AppState>,
    project_id: String,
    report_id: String,
) -> Result<PathBuf, AppError> {
    let state = state.inner().clone();
    blocking(move || project(&state, &project_id)?.accept_report(&report_id)).await
}

/// A new job id (a version 7 UUID, ordered by creation time).
fn new_job() -> JobId {
    JobId(Uuid::now_v7())
}

/// Analyses a project. The arguments are the fields of
/// [`blurrd_core::StartAnalysis`]. The detector options are not read: the
/// engine has no detector settings yet (plan section 18, M22).
#[tauri::command]
pub(crate) fn start_analysis(
    state: State<'_, AppState>,
    project_id: String,
    regions: RegionSettings,
    detector: DetectorOptions,
    verify: VerifySettings,
    on_event: Channel<JobEvent>,
) -> Result<JobId, AppError> {
    let _ = detector;
    let engine = state.engine()?;
    let project = project(&state, &project_id)?;
    let job = new_job();
    let work = tasks::analysis(engine, project, regions.enabled, verify.enabled);
    state.jobs.enqueue(job, Arc::new(on_event), work);
    Ok(job)
}

/// Blurs and encodes a project into a new file. The arguments are the
/// fields of [`blurrd_core::StartRender`].
#[tauri::command]
pub(crate) fn start_render(
    state: State<'_, AppState>,
    project_id: String,
    settings: ExportSettings,
    blur: BlurParams,
    enabled: RegionSet,
    on_event: Channel<JobEvent>,
) -> Result<JobId, AppError> {
    let engine = state.engine()?;
    let project = project(&state, &project_id)?;
    let request = ExportRequest {
        export: checked_export(&state, settings)?,
        blur,
        enabled,
    };
    let job = new_job();
    state.jobs.enqueue(
        job,
        Arc::new(on_event),
        tasks::render(engine, project, request),
    );
    Ok(job)
}

/// Verifies the ranges edits changed since the last verification.
#[tauri::command]
pub(crate) fn start_reverify(
    state: State<'_, AppState>,
    project_id: String,
    on_event: Channel<JobEvent>,
) -> Result<JobId, AppError> {
    let project = project(&state, &project_id)?;
    let job = new_job();
    state
        .jobs
        .enqueue(job, Arc::new(on_event), tasks::reverify(project));
    Ok(job)
}

/// Queues the export of several files; answers their job ids in order.
/// Every file is checked before any is queued.
#[tauri::command]
pub(crate) fn start_run(
    state: State<'_, AppState>,
    files: Vec<RunFile>,
    on_event: Channel<JobEvent>,
) -> Result<Vec<JobId>, AppError> {
    let engine = state.engine()?;
    let out: Arc<dyn EventOut> = Arc::new(on_event);
    let mut prepared = Vec::with_capacity(files.len());
    for file in files {
        let project = project(&state, &file.project_id)?;
        let request = ExportRequest {
            export: checked_export(&state, file.settings)?,
            blur: file.blur,
            enabled: file.enabled,
        };
        prepared.push((project, request, file.regions.enabled, file.verify.enabled));
    }
    Ok(prepared
        .into_iter()
        .map(|(project, request, regions, verify)| {
            let job = new_job();
            let work = tasks::run(Arc::clone(&engine), project, request, regions, verify);
            state.jobs.enqueue(job, Arc::clone(&out), work);
            job
        })
        .collect())
}

/// Cancels a running or waiting job.
#[tauri::command]
pub(crate) fn cancel_job(state: State<'_, AppState>, job: JobId) {
    state.jobs.cancel(job);
}

/// Pauses the job queue.
#[tauri::command]
pub(crate) fn pause_queue(state: State<'_, AppState>) {
    state.jobs.pause();
}

/// Resumes the job queue.
#[tauri::command]
pub(crate) fn resume_queue(state: State<'_, AppState>) {
    state.jobs.resume();
}

/// Every preset, built-ins first.
#[tauri::command]
pub(crate) async fn list_presets(state: State<'_, AppState>) -> Result<Vec<Preset>, AppError> {
    let state = state.inner().clone();
    blocking(move || state.presets.list()).await
}

/// Adds or replaces a user preset.
#[tauri::command]
pub(crate) async fn save_preset(
    state: State<'_, AppState>,
    preset: Preset,
) -> Result<Vec<Preset>, AppError> {
    let state = state.inner().clone();
    blocking(move || state.presets.save(preset)).await
}

/// Deletes a user preset.
#[tauri::command]
pub(crate) async fn delete_preset(
    state: State<'_, AppState>,
    id: String,
) -> Result<Vec<Preset>, AppError> {
    let state = state.inner().clone();
    blocking(move || state.presets.delete(&id)).await
}

/// Adds the presets of a chosen preset file.
#[tauri::command]
pub(crate) async fn import_preset(
    state: State<'_, AppState>,
    path: PathBuf,
) -> Result<Vec<Preset>, AppError> {
    let state = state.inner().clone();
    blocking(move || {
        let file = state.paths.existing(&path)?;
        state.presets.import(&file)
    })
    .await
}

/// Writes a preset to the file chosen in the save dialog.
#[tauri::command]
pub(crate) async fn export_preset(
    state: State<'_, AppState>,
    id: String,
    path: PathBuf,
    include_kept_names: bool,
) -> Result<(), AppError> {
    let state = state.inner().clone();
    blocking(move || {
        let file = state.paths.target(&path)?;
        state.presets.export(&id, &file, include_kept_names)
    })
    .await
}
