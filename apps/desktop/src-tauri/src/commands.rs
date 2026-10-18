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
    AppError, BlurParams, CacheInfo, Capabilities, Defaults, DensityLanes, ErrorKind,
    ExportSettings, FramePts, JobEvent, JobId, OutputFolder, OutputPreview, Override,
    OverridesSaved, Preset, Privacy, ProbeOutcome, Processing, ProjectSummary, RegionSet,
    RegionSettings, RunFile, StoredSettings, TrackSet, VerifyReport, VerifySettings,
};
use blurrd_engine::is_verified;
use tauri::ipc::Channel;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;
use uuid::Uuid;

use crate::jobs::EventOut;
use crate::media::video_url;
use crate::media_server::MediaServer;
use crate::output;
use crate::paths::UserPaths;
use crate::projects::OpenProject;
use crate::session::SessionEntry;
use crate::settings;
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
            Ok(files) => files.iter().map(|file| probe_file(state, file)).collect(),
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
    vec![probe_file(state, &checked)]
}

/// The outcome of one video file, with whether its analysis is cached.
fn probe_file(state: &AppState, path: &Path) -> ProbeOutcome {
    match blurrd_video::probe(path) {
        Ok(media) => ProbeOutcome::Media {
            path: path.to_path_buf(),
            media: Box::new(media),
            analyzed: state.projects.is_analysed(path),
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
        forget_media(state.media.as_deref(), &project_id);
        state.exact.close(&project_id);
    }
}

/// The summary of `project` with its media URLs.
fn summary(state: &AppState, project: &OpenProject) -> ProjectSummary {
    let proxy = project.dir().proxy();
    let (media_url, proxy_url) = media_urls(
        state.media.as_deref(),
        &state.paths,
        project.id(),
        project.source(),
        proxy.is_file().then_some(proxy.as_path()),
    );
    project.summary(media_url, proxy_url)
}

/// The media server id of the preview proxy of project `project_id`.
fn proxy_media_id(project_id: &str) -> String {
    format!("{project_id}/proxy")
}

/// The URLs of project `id`'s source and of its preview proxy (when it
/// exists), both from the media server when it runs. Without the server
/// they are asset protocol URLs, and the proxy, a file the backend
/// wrote, is added to that protocol's scope so it can be served.
fn media_urls(
    media: Option<&MediaServer>,
    paths: &UserPaths,
    id: &str,
    source: &Path,
    proxy: Option<&Path>,
) -> (String, Option<String>) {
    let proxy_url = proxy.map(|proxy| {
        if media.is_none() {
            paths.add_file(proxy);
        }
        video_url(media, &proxy_media_id(id), proxy)
    });
    (video_url(media, id, source), proxy_url)
}

/// Stops serving the source and the proxy of project `id`.
fn forget_media(media: Option<&MediaServer>, id: &str) {
    if let Some(media) = media {
        media.forget(id);
        media.forget(&proxy_media_id(id));
    }
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
/// [`blurrd_core::StartAnalysis`].
#[tauri::command]
pub(crate) fn start_analysis(
    state: State<'_, AppState>,
    project_id: String,
    regions: RegionSettings,
    verify: VerifySettings,
    on_event: Channel<JobEvent>,
) -> Result<JobId, AppError> {
    let engine = state.engine()?;
    let project = project(&state, &project_id)?;
    let job = new_job();
    let work = state
        .capabilities
        .after_probe(tasks::analysis(engine, project, regions, verify));
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
    let work = state
        .capabilities
        .after_probe(tasks::render(engine, project, request));
    state.jobs.enqueue(job, Arc::new(on_event), work);
    Ok(job)
}

/// Verifies the ranges edits changed since the last verification.
#[tauri::command]
pub(crate) fn start_reverify(
    state: State<'_, AppState>,
    project_id: String,
    on_event: Channel<JobEvent>,
) -> Result<JobId, AppError> {
    let engine = state.engine()?;
    let project = project(&state, &project_id)?;
    let job = new_job();
    let work = state
        .capabilities
        .after_probe(tasks::reverify(engine, project));
    state.jobs.enqueue(job, Arc::new(on_event), work);
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
        prepared.push((project, request, file.regions, file.verify));
    }
    Ok(prepared
        .into_iter()
        .map(|(project, request, regions, verify)| {
            let job = new_job();
            let work = tasks::run(Arc::clone(&engine), project, request, regions, verify);
            let work = state.capabilities.after_probe(work);
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
) -> Result<(), AppError> {
    let state = state.inner().clone();
    blocking(move || {
        let file = state.paths.target(&path)?;
        state.presets.export(&id, &file)
    })
    .await
}

/// The backend's settings sections (processing, defaults for new files,
/// privacy) as saved, with the processing the running engine uses.
#[tauri::command]
pub(crate) fn get_settings(app: AppHandle, state: State<'_, AppState>) -> StoredSettings {
    settings::get(&app, &state.processing)
}

/// Saves the processing settings; the engine uses them from the next
/// start.
#[tauri::command]
pub(crate) fn save_processing(app: AppHandle, processing: Processing) -> Result<(), AppError> {
    let processing = settings::checked_processing(processing)?;
    settings::save_processing(&app, &processing)
}

/// Saves the defaults for new files; answers them as stored (a chosen
/// output folder in its canonical form).
#[tauri::command]
pub(crate) async fn save_defaults(
    app: AppHandle,
    state: State<'_, AppState>,
    defaults: Defaults,
) -> Result<Defaults, AppError> {
    let state = state.inner().clone();
    blocking(move || {
        let defaults =
            settings::accepted_defaults(&state.paths, &state.chosen_folder_record, defaults)?;
        settings::save_defaults(&app, &defaults)?;
        Ok(defaults)
    })
    .await
}

/// Saves the privacy settings and applies the log level at once.
#[tauri::command]
pub(crate) fn save_privacy(app: AppHandle, privacy: Privacy) -> Result<(), AppError> {
    settings::save_privacy(&app, privacy)?;
    settings::apply_log_level(privacy.log_level);
    Ok(())
}

/// The cache and log folders and how much the cached analyses take.
#[tauri::command]
pub(crate) async fn get_cache_info(state: State<'_, AppState>) -> Result<CacheInfo, AppError> {
    let state = state.inner().clone();
    blocking(move || Ok(cache_info(&state))).await
}

/// Deletes the cached analyses (their edits and verification reports go
/// with them) except those of the open projects and of `keep` (the queued
/// files); answers what is left.
#[tauri::command]
pub(crate) async fn clear_cache(
    state: State<'_, AppState>,
    keep: Vec<PathBuf>,
) -> Result<CacheInfo, AppError> {
    let state = state.inner().clone();
    blocking(move || {
        let mut ids = state.projects.open_ids();
        ids.extend(keep.iter().filter_map(|path| {
            let path = state.paths.existing(path).ok()?;
            Some(
                blurrd_engine::cache::fingerprint(&path)
                    .ok()?
                    .blake3_head_tail,
            )
        }));
        let failed = settings::clear_projects(&state.cache_dir(), &ids);
        if failed > 0 {
            tauri_plugin_log::log::warn!("{failed} cached analyses could not be deleted");
        }
        Ok(cache_info(&state))
    })
    .await
}

/// What `get_cache_info` answers.
fn cache_info(state: &AppState) -> CacheInfo {
    let cache_dir = state.cache_dir();
    let (projects, project_bytes) = settings::project_usage(&cache_dir);
    CacheInfo {
        cache_dir,
        projects,
        project_bytes,
        log_dir: state.log_dir.to_path_buf(),
    }
}

/// Opens the log folder in Explorer (created first when no log exists
/// yet).
#[tauri::command]
pub(crate) fn open_log_dir(app: AppHandle, state: State<'_, AppState>) -> Result<(), AppError> {
    let folder = state.log_dir.to_path_buf();
    let failed = |detail: String| {
        AppError::new(ErrorKind::Io, "The log folder could not be opened.").with_detail(detail)
    };
    std::fs::create_dir_all(&folder).map_err(|e| failed(format!("{}: {e}", folder.display())))?;
    app.opener()
        .open_path(folder.to_string_lossy(), None::<&str>)
        .map_err(|e| failed(e.to_string()))
}

/// Forgets the cached decoder benchmark and encoder probe and probes this
/// PC again; refused while a probe runs or a job runs or waits (the probe
/// encodes on the GPU).
#[tauri::command]
pub(crate) fn refresh_capabilities(state: State<'_, AppState>) -> Result<(), AppError> {
    state.reprobe_capabilities()
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::path::PathBuf;

    use super::{forget_media, media_urls};
    use crate::media::asset_url;
    use crate::media_server::MediaServer;
    use crate::paths::UserPaths;
    use crate::paths::tests::TestScope;

    /// A folder with a source video and its proxy.
    fn files(name: &str) -> (PathBuf, PathBuf, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("blurrd-commands-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("clip.mp4");
        let proxy = dir.join("proxy.mp4");
        std::fs::write(&source, b"source").unwrap();
        std::fs::write(&proxy, b"proxy").unwrap();
        (dir, source, proxy)
    }

    /// The status and body of a GET of `url`.
    fn get(url: &str) -> (u16, String) {
        let (address, resource) = url
            .strip_prefix("http://")
            .unwrap()
            .split_once('/')
            .unwrap();
        let mut stream = TcpStream::connect(address).unwrap();
        write!(
            stream,
            "GET /{resource} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut answer = String::new();
        stream.read_to_string(&mut answer).unwrap();
        let status = answer.split_whitespace().nth(1).unwrap().parse().unwrap();
        let body = answer.split_once("\r\n\r\n").unwrap().1.to_owned();
        (status, body)
    }

    #[test]
    fn source_and_proxy_play_from_the_media_server_until_closed() {
        let (dir, source, proxy) = files("server");
        let server = MediaServer::start("http://tauri.localhost").unwrap();
        let paths = UserPaths::new(Box::new(TestScope::default()));

        let (media_url, proxy_url) =
            media_urls(Some(&server), &paths, "abc", &source, Some(&proxy));
        let proxy_url = proxy_url.unwrap();
        assert!(media_url.starts_with("http://127.0.0.1:"), "{media_url}");
        assert!(proxy_url.starts_with("http://127.0.0.1:"), "{proxy_url}");
        assert_ne!(media_url, proxy_url);
        assert_eq!(get(&media_url), (200, "source".to_owned()));
        assert_eq!(get(&proxy_url), (200, "proxy".to_owned()));
        // The server serves the proxy, so the asset scope does not grow.
        assert!(paths.existing(&proxy).is_err());

        // No proxy yet: no proxy URL.
        let (_, none) = media_urls(Some(&server), &paths, "abc", &source, None);
        assert_eq!(none, None);

        // Closing the project stops serving both.
        forget_media(Some(&server), "abc");
        assert_eq!(get(&media_url).0, 404);
        assert_eq!(get(&proxy_url).0, 404);
        // Other projects are not affected.
        let (other, _) = media_urls(Some(&server), &paths, "other", &source, None);
        forget_media(Some(&server), "abc");
        assert_eq!(get(&other).0, 200);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn without_the_server_source_and_proxy_use_the_asset_protocol() {
        let (dir, source, proxy) = files("asset");
        let paths = UserPaths::new(Box::new(TestScope::default()));

        let (media_url, proxy_url) = media_urls(None, &paths, "abc", &source, Some(&proxy));
        assert_eq!(media_url, asset_url(&source));
        assert_eq!(proxy_url, Some(asset_url(&proxy)));
        // The asset protocol serves only files in its scope.
        assert!(paths.existing(&proxy).is_ok());
        assert!(paths.existing(&source).is_err());
        // Nothing to forget without the server.
        forget_media(None, "abc");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
