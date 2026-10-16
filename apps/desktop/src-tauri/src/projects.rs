//! The projects the frontend opened: one per source video, named by the
//! video's fingerprint (architecture plan 6.2).
//!
//! The project file in the engine's cache is the single source of truth:
//! every read goes to the file, so the results of an analysis that just
//! finished are seen at once, and every edit is a read-modify-write of
//! the file under the project's lock. A video that was not analysed yet
//! has no project file; it reads as an empty project at revision 0.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use blurrd_core::{
    AppError, BlurParams, DensityLanes, ErrorKind, Fingerprint, FramePts, FrameRange, MediaInfo,
    Override, OverridesSaved, Project, ProjectSummary, ProjectVerify, RegionSet, ThumbnailStrip,
    TrackSet, VerifyReport, VerifySummary, schema,
};
use blurrd_engine::UNVERIFIED_SUFFIX;
use blurrd_engine::cache::{self, ProjectDir, THUMBNAIL_INTERVAL_S};
use blurrd_video::PtsTable;

/// Lines of the reference HUD the detectors work in; the HUD scale of a
/// video that was not analysed yet is its height over this (the engine's
/// assumed geometry before calibration).
const REFERENCE_HEIGHT: f32 = 1080.0;

/// The projects opened in this session.
#[derive(Debug)]
pub(crate) struct Projects {
    cache: PathBuf,
    open: Mutex<HashMap<String, Arc<OpenProject>>>,
}

impl Projects {
    /// Projects whose files live in the engine cache `cache`.
    pub(crate) fn new(cache: PathBuf) -> Self {
        Self {
            cache,
            open: Mutex::new(HashMap::new()),
        }
    }

    /// Opens the project of the video at `source` (an absolute, checked
    /// path): fingerprints and probes the file. Opening a file again
    /// replaces the earlier entry, so a changed file is noticed.
    ///
    /// # Errors
    ///
    /// The fingerprint's I/O errors and the probe's media errors.
    pub(crate) fn open(&self, source: &Path) -> Result<Arc<OpenProject>, AppError> {
        let fingerprint = cache::fingerprint(source)?;
        let info = blurrd_video::probe(source)?;
        let project = Arc::new(OpenProject {
            id: fingerprint.blake3_head_tail.clone(),
            dir: ProjectDir::new(&self.cache, &fingerprint),
            source: source.to_path_buf(),
            fingerprint,
            info,
            pts: Mutex::new(None),
            edit: Mutex::new(()),
            last_output: Mutex::new(None),
        });
        lock(&self.open).insert(project.id.clone(), Arc::clone(&project));
        Ok(project)
    }

    /// The open project `id`.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidProject`] when no project with that id was
    /// opened in this session.
    pub(crate) fn get(&self, id: &str) -> Result<Arc<OpenProject>, AppError> {
        lock(&self.open).get(id).cloned().ok_or_else(|| {
            AppError::new(ErrorKind::InvalidProject, "The project is not open.")
                .with_detail(format!("project {id}"))
        })
    }

    /// Closes the open project `id` (the frontend removed its file);
    /// answers whether it was open.
    pub(crate) fn close(&self, id: &str) -> bool {
        lock(&self.open).remove(id).is_some()
    }

    /// The fingerprint of the open project of `source`, if there is one.
    pub(crate) fn fingerprint_of(&self, source: &Path) -> Option<Fingerprint> {
        lock(&self.open)
            .values()
            .find(|project| project.source == source)
            .map(|project| project.fingerprint.clone())
    }
}

/// One opened video and its cache folder.
#[derive(Debug)]
pub(crate) struct OpenProject {
    id: String,
    source: PathBuf,
    fingerprint: Fingerprint,
    info: MediaInfo,
    dir: ProjectDir,
    /// The PTS table, once read or measured.
    pts: Mutex<Option<Arc<PtsTable>>>,
    /// Held while the project file is read and written by an edit.
    edit: Mutex<()>,
    /// The last output a render of this session wrote.
    last_output: Mutex<Option<PathBuf>>,
}

impl OpenProject {
    /// The project id (the fingerprint's hash).
    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    /// The source video.
    pub(crate) fn source(&self) -> &Path {
        &self.source
    }

    /// The source's media facts.
    pub(crate) fn info(&self) -> &MediaInfo {
        &self.info
    }

    /// The cache folder.
    pub(crate) fn dir(&self) -> &ProjectDir {
        &self.dir
    }

    /// The analysed project, if the cache holds one for this file.
    pub(crate) fn project(&self) -> Option<Project> {
        self.dir
            .read_project()
            .filter(|project| cache::same_file(&project.media.fingerprint, &self.fingerprint))
    }

    /// The analysed project.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidProject`] when the file was not analysed yet.
    pub(crate) fn analysed(&self) -> Result<Project, AppError> {
        self.project().ok_or_else(|| {
            AppError::new(ErrorKind::InvalidProject, "Analyze this file first.")
                .with_detail(self.source.display().to_string())
        })
    }

    /// The summary the preview needs; `media_url` and `proxy_url` are the
    /// URLs of the source and of the proxy (when it exists).
    pub(crate) fn summary(&self, media_url: String, proxy_url: Option<String>) -> ProjectSummary {
        let project = self.project();
        let thumbs = self.thumbnails(project.as_ref());
        match project {
            Some(project) => ProjectSummary {
                id: self.id.clone(),
                media: self.info.clone(),
                media_url,
                proxy_url,
                sy: project.geometry.sy,
                blur: project.blur,
                enabled: project.detector.regions,
                analyzed: true,
                verify: project.verify,
                thumbs,
            },
            None => ProjectSummary {
                id: self.id.clone(),
                media: self.info.clone(),
                media_url,
                proxy_url,
                sy: self.info.height as f32 / REFERENCE_HEIGHT,
                blur: BlurParams::default(),
                enabled: RegionSet::all(),
                analyzed: false,
                verify: ProjectVerify::default(),
                thumbs,
            },
        }
    }

    /// The thumbnails the analysis wrote: consecutive files from index 0,
    /// one every [`THUMBNAIL_INTERVAL_S`] seconds.
    fn thumbnails(&self, project: Option<&Project>) -> Option<ThumbnailStrip> {
        let project = project?;
        let count = (0..u32::MAX)
            .take_while(|&index| self.dir.thumbnail(index).is_file())
            .count();
        let count = u32::try_from(count).ok().filter(|&count| count > 0)?;
        let fps = project.media.fps.as_f64();
        let every = (THUMBNAIL_INTERVAL_S * fps).round().max(1.0) as u32;
        Some(ThumbnailStrip { every, count })
    }

    /// The tracks, edits and edit bindings (empty before the analysis).
    pub(crate) fn tracks(&self) -> TrackSet {
        match self.project() {
            Some(project) => {
                let bindings = project.plan().bindings();
                TrackSet {
                    revision: project.revision,
                    tracks: project.tracks,
                    overrides: project.overrides,
                    bindings,
                    health: Vec::new(),
                }
            }
            None => TrackSet {
                revision: 0,
                tracks: Vec::new(),
                overrides: Vec::new(),
                bindings: Vec::new(),
                health: Vec::new(),
            },
        }
    }

    /// Per-region box coverage in `buckets` parts of the timeline.
    pub(crate) fn density(&self, buckets: u32) -> DensityLanes {
        match self.project() {
            Some(project) => project.plan().density(buckets),
            None => DensityLanes {
                frame_count: self.info.frame_count.unwrap_or(0),
                buckets: buckets.min(self.info.frame_count.unwrap_or(0)),
                lanes: Vec::new(),
            },
        }
    }

    /// The PTS table: the analysis's `pts.bin` when it belongs to the
    /// project, otherwise one pass over the container's packets
    /// (remembered until the next analysis).
    ///
    /// # Errors
    ///
    /// The demuxer's errors when the table must be measured.
    pub(crate) fn pts(&self) -> Result<Arc<PtsTable>, AppError> {
        if let Some(pts) = lock(&self.pts).as_ref() {
            return Ok(Arc::clone(pts));
        }
        let cached = self.project().and_then(|project| {
            self.dir.read_pts().filter(|pts| {
                cache::pts_sha256(pts).eq_ignore_ascii_case(&project.media.pts_sha256)
            })
        });
        let table = Arc::new(match cached {
            Some(table) => table,
            None => blurrd_video::pts_table(&self.source)?,
        });
        *lock(&self.pts) = Some(Arc::clone(&table));
        Ok(table)
    }

    /// The PTS table in its IPC form.
    ///
    /// # Errors
    ///
    /// See [`OpenProject::pts`].
    pub(crate) fn frame_pts(&self) -> Result<FramePts, AppError> {
        let table = self.pts()?;
        Ok(FramePts {
            time_base: table.time_base(),
            pts: table.as_slice().to_vec(),
        })
    }

    /// Forgets the remembered PTS table (an analysis wrote a new one).
    pub(crate) fn forget_pts(&self) {
        *lock(&self.pts) = None;
    }

    /// The last verification report, if there is one.
    ///
    /// # Errors
    ///
    /// The I/O and schema errors of reading it.
    pub(crate) fn verify_report(&self) -> Result<Option<VerifyReport>, AppError> {
        match self.project() {
            Some(project) => self.report_of(&project),
            None => Ok(None),
        }
    }

    /// The verification report `project` (this file's analysed project)
    /// names, if it names one.
    ///
    /// # Errors
    ///
    /// The I/O and schema errors of reading it.
    pub(crate) fn report_of(&self, project: &Project) -> Result<Option<VerifyReport>, AppError> {
        let Some(name) = &project.verify.report else {
            return Ok(None);
        };
        read_report(&self.report_path(name)?).map(Some)
    }

    /// Replaces the edits with `overrides` (the full list) when the
    /// project is still at `base_rev`. The frames of every added, removed
    /// or changed edit (old and new range) join the unverified ranges.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::StaleRevision`] when the project changed since
    /// `base_rev`, [`ErrorKind::InvalidProject`] for invalid edits or a
    /// file that was not analysed yet, I/O errors of the project file.
    pub(crate) fn set_overrides(
        &self,
        base_rev: u64,
        overrides: Vec<Override>,
    ) -> Result<OverridesSaved, AppError> {
        let _edit = lock(&self.edit);
        let mut project = self.analysed()?;
        if project.revision != base_rev {
            return Err(AppError::new(
                ErrorKind::StaleRevision,
                "The project changed in the meantime.",
            )
            .with_detail(format!(
                "the project is at revision {}, not {base_rev}",
                project.revision
            )));
        }
        if let Some(problem) = invalid_edit(&overrides, &project) {
            return Err(
                AppError::new(ErrorKind::InvalidProject, "The edit is invalid.")
                    .with_detail(problem),
            );
        }
        let mut ranges = project.verify.unverified_ranges.clone();
        ranges.extend(touched_ranges(&project.overrides, &overrides));
        project.verify.unverified_ranges = merge_ranges(ranges);
        project.overrides = overrides;
        project.revision += 1;
        cache::write_project(&project, &self.dir.project())?;
        Ok(OverridesSaved {
            revision: project.revision,
            unverified_ranges: project.verify.unverified_ranges,
        })
    }

    /// Records the output a render of this project wrote.
    pub(crate) fn set_last_output(&self, output: PathBuf) {
        *lock(&self.last_output) = Some(output);
    }

    /// Accepts the open leaks of report `report_id` and renames the last
    /// `_UNVERIFIED` output of this session; answers the new path.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::NotFound`] when `report_id` is not the project's
    /// report, [`ErrorKind::InvalidProject`] when there is no unverified
    /// output to rename, I/O and schema errors of the files.
    pub(crate) fn accept_report(&self, report_id: &str) -> Result<PathBuf, AppError> {
        let _edit = lock(&self.edit);
        let name = self
            .project()
            .and_then(|project| project.verify.report)
            .filter(|name| name == report_id)
            .ok_or_else(|| {
                AppError::new(ErrorKind::NotFound, "The report does not exist.")
                    .with_detail(report_id.to_owned())
            })?;
        let output = lock(&self.last_output).clone();
        let (output, renamed) = output
            .and_then(|output| verified_name(&output).map(|renamed| (output, renamed)))
            .ok_or_else(|| {
                AppError::new(
                    ErrorKind::InvalidProject,
                    "There is no unverified output to rename.",
                )
            })?;
        let path = self.report_path(&name)?;
        let mut report = read_report(&path)?;
        for leak in &mut report.leaks {
            if leak.status == blurrd_core::LeakStatus::Open {
                leak.status = blurrd_core::LeakStatus::Accepted;
            }
        }
        report.summary = VerifySummary::compute(&report.leaks, &report.per_region);
        write_report(&report, &path)?;
        std::fs::rename(&output, &renamed).map_err(|e| {
            AppError::new(ErrorKind::Io, "The output could not be renamed.")
                .with_detail(format!("{}: {e}", output.display()))
        })?;
        *lock(&self.last_output) = Some(renamed.clone());
        Ok(renamed)
    }

    /// The path of report `name` inside the project folder.
    fn report_path(&self, name: &str) -> Result<PathBuf, AppError> {
        let plain = Path::new(name).file_name().is_some_and(|file| file == name);
        if plain {
            Ok(self.dir.path().join(name))
        } else {
            Err(
                AppError::new(ErrorKind::InvalidProject, "The report name is invalid.")
                    .with_detail(name.to_owned()),
            )
        }
    }
}

/// `output` without its `_UNVERIFIED` suffix, if it has one.
fn verified_name(output: &Path) -> Option<PathBuf> {
    let stem = output.file_stem()?.to_str()?;
    let base = stem.strip_suffix(UNVERIFIED_SUFFIX)?;
    let mut name = base.to_owned();
    if let Some(extension) = output.extension() {
        name.push('.');
        name.push_str(&extension.to_string_lossy());
    }
    Some(output.with_file_name(name))
}

fn read_report(path: &Path) -> Result<VerifyReport, AppError> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        AppError::new(ErrorKind::Io, "The verification report could not be read.")
            .with_detail(format!("{}: {e}", path.display()))
    })?;
    Ok(schema::from_json(&text)?)
}

fn write_report(report: &VerifyReport, path: &Path) -> Result<(), AppError> {
    let text = schema::to_json(report)?;
    std::fs::write(path, text).map_err(|e| {
        AppError::new(ErrorKind::Io, "The verification report could not be saved.")
            .with_detail(format!("{}: {e}", path.display()))
    })
}

/// Why `overrides` cannot be saved for `project`, if they cannot: a uid
/// listed twice, frames outside the video, a box outside the frame.
fn invalid_edit(overrides: &[Override], project: &Project) -> Option<String> {
    let media = &project.media;
    let mut uids = std::collections::HashSet::new();
    for edit in overrides {
        let uid = edit.uid();
        if !uids.insert(uid) {
            return Some(format!("edit {uid} is listed twice"));
        }
        let (f0, f1) = edit.frames();
        if f0 > f1 || f1 >= media.frame_count {
            return Some(format!("edit {uid} has frames outside the video"));
        }
        let rects = match *edit {
            Override::Add { rect, .. } => vec![rect],
            Override::Suppress { anchor, .. } => vec![anchor],
            Override::Adjust { anchor, rect, .. } => vec![anchor, rect],
            _ => Vec::new(),
        };
        if !rects
            .iter()
            .all(|rect| rect.is_inside(media.width, media.height))
        {
            return Some(format!("edit {uid} has a box outside the frame"));
        }
    }
    None
}

/// The frame ranges of every edit that was added, removed or changed
/// between `before` and `after` (old and new range of a changed edit).
fn touched_ranges(before: &[Override], after: &[Override]) -> Vec<FrameRange> {
    let old: HashMap<_, _> = before.iter().map(|edit| (edit.uid(), edit)).collect();
    let new: HashMap<_, _> = after.iter().map(|edit| (edit.uid(), edit)).collect();
    let range = |edit: &Override| {
        let (f0, f1) = edit.frames();
        FrameRange { f0, f1 }
    };
    let mut ranges = Vec::new();
    for edit in before {
        if new.get(&edit.uid()) != Some(&edit) {
            ranges.push(range(edit));
        }
    }
    for edit in after {
        if old.get(&edit.uid()) != Some(&edit) {
            ranges.push(range(edit));
        }
    }
    ranges
}

/// Sorts `ranges` and merges overlapping and adjacent ones.
fn merge_ranges(mut ranges: Vec<FrameRange>) -> Vec<FrameRange> {
    ranges.sort_unstable_by_key(|range| range.f0);
    let mut merged: Vec<FrameRange> = Vec::with_capacity(ranges.len());
    for range in ranges {
        match merged.last_mut() {
            Some(last) if range.f0 <= last.f1.saturating_add(1) => {
                last.f1 = last.f1.max(range.f1);
            }
            _ => merged.push(range),
        }
    }
    merged
}

/// Locks `mutex`, recovering the data of a poisoned lock (a panicking
/// job thread never leaves this state half-written).
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use blurrd_core::{FrameRange, Override, RectI, Region};
    use uuid::Uuid;

    use super::{merge_ranges, touched_ranges, verified_name};

    fn add(uid: u128, f0: u32, f1: u32) -> Override {
        Override::Add {
            uid: Uuid::from_u128(uid),
            region: Region::Manual,
            f0,
            f1,
            rect: RectI::new(0, 0, 10, 10).unwrap(),
        }
    }

    fn range(f0: u32, f1: u32) -> FrameRange {
        FrameRange { f0, f1 }
    }

    #[test]
    fn ranges_merge_when_they_overlap_or_touch() {
        let merged = merge_ranges(vec![range(10, 20), range(0, 4), range(21, 25), range(5, 5)]);
        assert_eq!(merged, vec![range(0, 5), range(10, 25)]);
    }

    #[test]
    fn touched_ranges_cover_added_removed_and_changed_edits() {
        let before = vec![add(1, 0, 9), add(2, 100, 110), add(3, 50, 60)];
        let after = vec![add(1, 0, 9), add(2, 200, 210), add(4, 300, 300)];
        let mut touched = touched_ranges(&before, &after);
        touched.sort_unstable_by_key(|r| r.f0);
        assert_eq!(
            touched,
            vec![
                range(50, 60),
                range(100, 110),
                range(200, 210),
                range(300, 300)
            ]
        );
        assert_eq!(touched_ranges(&before, &before), vec![]);
    }

    #[test]
    fn unverified_outputs_lose_their_suffix() {
        assert_eq!(
            verified_name(Path::new(r"C:\out\clip_blurrd_UNVERIFIED.mp4")),
            Some(PathBuf::from(r"C:\out\clip_blurrd.mp4"))
        );
        assert_eq!(verified_name(Path::new(r"C:\out\clip_blurrd.mp4")), None);
    }
}
