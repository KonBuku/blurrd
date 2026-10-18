//! Output naming and export estimates (architecture plan 6.4).
//!
//! The output goes into the source's folder or the chosen one, named by
//! the template (`{name}` is the source's file stem) with the container's
//! extension. The render adds `_UNVERIFIED` to the name while the project
//! is not verified, so a conflict exists when either name is taken. A
//! conflict adds a number (`clip_blurrd (2).mp4`), overwrites or skips,
//! as the settings say; the source itself is never overwritten.

use std::path::{Path, PathBuf};

use blurrd_core::{
    AppError, AudioMode, ConflictPolicy, Container, ErrorKind, ExportSettings, MediaInfo,
    OutputFolder, OutputPreview, RateControl,
};

use blurrd_engine::unverified_name;

/// Characters Windows does not allow in file names.
const FORBIDDEN: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

/// Bit rate assumed for a source that does not state one, in bit/s.
const DEFAULT_BITRATE: f64 = 20_000_000.0;
/// Bit rate of re-encoded or copied audio when the source does not state
/// one, in bit/s.
const DEFAULT_AUDIO_BITRATE: f64 = 192_000.0;
/// Export speed of a 1080p H.264 export at preset 3 on the reference
/// machine (architecture plan 11.4: a 60 s 1080p60 run in about 6 s).
/// The estimate scales it with the picture size and the preset until the
/// benchmark measures this machine.
const REFERENCE_FPS: f64 = 600.0;
/// Pixels of the reference picture.
const REFERENCE_PIXELS: f64 = 1920.0 * 1080.0;

/// Where an export goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OutputTarget {
    /// Write to this path (the render adds `_UNVERIFIED` when needed).
    Write(PathBuf),
    /// The file exists and the conflict policy is Skip.
    Skip(PathBuf),
}

/// The output of exporting `source` with `export`, before the render's
/// `_UNVERIFIED` naming; `verified` says which name the render will use.
///
/// # Errors
///
/// [`ErrorKind::InvalidProject`] for a template that names no valid file
/// or an output that would replace the source.
pub(crate) fn output_target(
    source: &Path,
    export: &ExportSettings,
    verified: bool,
) -> Result<OutputTarget, AppError> {
    let folder = match &export.output.folder {
        OutputFolder::Folder(folder) => folder.clone(),
        _ => source.parent().map(Path::to_path_buf).unwrap_or_default(),
    };
    let stem = source
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let base = export.output.template.replace("{name}", &stem);
    if !is_file_name(&base) {
        return Err(
            invalid("The file name template does not name a valid file.").with_detail(base),
        );
    }
    let extension = extension(export.container, source);
    let name = |n: u32| {
        let numbered = if n < 2 {
            base.clone()
        } else {
            format!("{base} ({n})")
        };
        folder.join(match &extension {
            Some(ext) => format!("{numbered}.{ext}"),
            None => numbered,
        })
    };
    let written = |path: &Path| {
        if verified {
            path.to_path_buf()
        } else {
            unverified_name(path)
        }
    };
    let taken = |path: &Path| path.exists() || unverified_name(path).exists();
    let is_source = |path: &Path| same_path(&written(path), source);

    let first = name(1);
    match export.output.on_conflict {
        ConflictPolicy::Overwrite | ConflictPolicy::Skip if is_source(&first) => {
            Err(invalid("The output would replace the source.")
                .with_detail(first.display().to_string()))
        }
        ConflictPolicy::Skip if taken(&first) => Ok(OutputTarget::Skip(written(&first))),
        ConflictPolicy::Overwrite | ConflictPolicy::Skip => Ok(OutputTarget::Write(first)),
        _ => {
            let free = (1..=u32::MAX)
                .map(name)
                .find(|path| !taken(path) && !is_source(path))
                .unwrap_or(first);
            Ok(OutputTarget::Write(free))
        }
    }
}

/// Whether `base` can be a Windows file name (before the extension).
fn is_file_name(base: &str) -> bool {
    !(base.trim().is_empty() || base.contains(FORBIDDEN) || base.chars().any(char::is_control))
}

/// Whether the file name template `template` names a valid file for any
/// source name (`{name}` stands for the source's file stem).
pub(crate) fn is_valid_template(template: &str) -> bool {
    is_file_name(&template.replace("{name}", "clip"))
}

/// What `get_output_preview` answers: the file an export would write (with
/// its `_UNVERIFIED` name when the project is not verified) and rough
/// estimates of its size and of the export's duration.
///
/// # Errors
///
/// See [`output_target`].
pub(crate) fn preview(
    source: &Path,
    info: &MediaInfo,
    export: &ExportSettings,
    verified: bool,
) -> Result<OutputPreview, AppError> {
    let path = match output_target(source, export, verified)? {
        OutputTarget::Write(path) if !verified => unverified_name(&path),
        OutputTarget::Write(path) | OutputTarget::Skip(path) => path,
    };
    let audio = match export.audio {
        AudioMode::Remove => 0.0,
        AudioMode::Aac(kbps) => f64::from(kbps) * 1000.0,
        _ => info
            .audio
            .iter()
            .map(|stream| stream.bitrate.map_or(DEFAULT_AUDIO_BITRATE, |b| b as f64))
            .sum(),
    };
    let bits = (video_bitrate(export, info) + audio) * info.duration_s.max(0.0);
    let frames = info
        .frame_count
        .map_or(info.duration_s * info.fps.as_f64(), f64::from);
    let pixels = f64::from(info.width) * f64::from(info.height);
    let preset = 1.0 + (3.0 - f64::from(export.preset)) * 0.15;
    let fps = REFERENCE_FPS * (REFERENCE_PIXELS / pixels.max(1.0)) * preset.max(0.1);
    Ok(OutputPreview {
        path,
        est_size: (bits / 8.0).round() as u64,
        est_seconds: frames / fps,
    })
}

/// The video bit rate an export aims for, in bit/s.
fn video_bitrate(export: &ExportSettings, info: &MediaInfo) -> f64 {
    let source = info.bitrate.map_or(DEFAULT_BITRATE, |b| b as f64);
    match export.rate {
        RateControl::Vbr { target_kbps, .. } => f64::from(target_kbps) * 1000.0,
        // Every 6 CQ steps halve the bit rate; CQ 19 is about the source's.
        RateControl::Cq(cq) => source * 2f64.powf((19.0 - f64::from(cq)) / 6.0),
        _ => source,
    }
}

/// The output's extension: the container's, or the source's.
fn extension(container: Container, source: &Path) -> Option<String> {
    match container {
        Container::Mp4 => Some("mp4".to_owned()),
        Container::Mkv => Some("mkv".to_owned()),
        Container::Mov => Some("mov".to_owned()),
        _ => source
            .extension()
            .map(|ext| ext.to_string_lossy().into_owned()),
    }
}

/// Whether two paths name the same file (Windows compares names without
/// regard to case).
fn same_path(a: &Path, b: &Path) -> bool {
    a.to_string_lossy()
        .eq_ignore_ascii_case(&b.to_string_lossy())
}

fn invalid(message: &str) -> AppError {
    AppError::new(ErrorKind::InvalidProject, message)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use blurrd_core::{ConflictPolicy, Container, ExportSettings, OutputFolder};

    use super::{OutputTarget, output_target};

    struct Folder(PathBuf);

    impl Folder {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("blurrd-out-{name}-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("clip.mp4"), b"source").unwrap();
            Self(dir)
        }

        fn source(&self) -> PathBuf {
            self.0.join("clip.mp4")
        }

        fn touch(&self, name: &str) {
            std::fs::write(self.0.join(name), b"x").unwrap();
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn written(target: OutputTarget) -> PathBuf {
        match target {
            OutputTarget::Write(path) => path,
            OutputTarget::Skip(path) => panic!("skipped {}", path.display()),
        }
    }

    fn file_name(path: &Path) -> String {
        path.file_name().unwrap().to_string_lossy().into_owned()
    }

    #[test]
    fn names_follow_the_template_and_number_conflicts() {
        let folder = Folder::new("number");
        let export = ExportSettings::default();
        let first = written(output_target(&folder.source(), &export, false).unwrap());
        assert_eq!(file_name(&first), "clip_blurrd.mp4");
        // The unverified name of the first output is taken.
        folder.touch("clip_blurrd_UNVERIFIED.mp4");
        let second = written(output_target(&folder.source(), &export, true).unwrap());
        assert_eq!(file_name(&second), "clip_blurrd (2).mp4");
    }

    #[test]
    fn the_source_is_never_replaced() {
        let folder = Folder::new("source");
        let mut export = ExportSettings::default();
        export.output.template = "{name}".to_owned();
        let numbered = written(output_target(&folder.source(), &export, true).unwrap());
        assert_eq!(file_name(&numbered), "clip (2).mp4");
        export.output.on_conflict = ConflictPolicy::Overwrite;
        assert!(output_target(&folder.source(), &export, true).is_err());
    }

    #[test]
    fn skip_and_custom_folders_and_containers() {
        let folder = Folder::new("skip");
        folder.touch("clip_blurrd.mkv");
        let mut export = ExportSettings {
            container: Container::Mkv,
            ..ExportSettings::default()
        };
        export.output.on_conflict = ConflictPolicy::Skip;
        assert!(matches!(
            output_target(&folder.source(), &export, true).unwrap(),
            OutputTarget::Skip(_)
        ));
        let other = Folder::new("other");
        export.output.folder = OutputFolder::Folder(other.0.clone());
        let path = written(output_target(&folder.source(), &export, true).unwrap());
        assert_eq!(path, other.0.join("clip_blurrd.mkv"));
    }

    #[test]
    fn templates_must_name_a_file() {
        let folder = Folder::new("template");
        let mut export = ExportSettings::default();
        export.output.template = "..\\{name}".to_owned();
        assert!(output_target(&folder.source(), &export, true).is_err());
    }
}
