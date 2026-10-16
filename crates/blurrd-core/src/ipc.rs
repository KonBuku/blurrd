//! Payloads of the desktop app's IPC commands (architecture plan 9.1 and
//! 9.2) that are not stored documents.
//!
//! The desktop shell answers its commands with these types and the
//! frontend imports their generated TypeScript bindings, so both sides
//! share one definition. Documents (projects, settings, presets, reports)
//! and job events live in their own modules.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::blur_params::BlurParams;
use crate::error::AppError;
use crate::geometry::RectI;
use crate::media::MediaInfo;
use crate::overrides::{Override, OverrideBinding};
use crate::plan::Track;
use crate::project::ProjectVerify;
use crate::region::{Region, RegionSet};
use crate::settings::{Sensitivity, TemporalParams};
use crate::time::{FrameRange, Rational};

/// What `probe_media` answers for one file: its media facts, or why it
/// cannot be used. A folder named in the request expands to one outcome
/// per video inside it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum ProbeOutcome {
    /// A usable video.
    Media {
        /// The file.
        path: PathBuf,
        /// Its media facts (boxed: much larger than an error).
        media: Box<MediaInfo>,
    },
    /// A file that is not a usable video.
    Error {
        /// The file.
        path: PathBuf,
        /// Why it cannot be used.
        error: AppError,
    },
}

/// The timeline thumbnails of a project: one every `every` frames,
/// `count` in total, served as `blurrd://thumb/<project>/<index>.jpg`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ThumbnailStrip {
    /// Frames between two thumbnails.
    pub every: u32,
    /// Number of thumbnails.
    pub count: u32,
}

/// An opened project as the preview needs it (`open_project`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ProjectSummary {
    /// The project id: the source fingerprint's hash, used in `blurrd://`
    /// URLs and in every command that takes a project.
    pub id: String,
    /// The source video.
    pub media: MediaInfo,
    /// URL the preview plays the source from (asset protocol or the
    /// loopback media server).
    pub media_url: String,
    /// URL of the H.264 preview proxy made during analysis, for videos the
    /// player cannot play.
    pub proxy_url: Option<String>,
    /// HUD scale of the video (`HudGeometry::sy`), for the blur sizes.
    pub sy: f32,
    /// Blur parameters of the project.
    pub blur: BlurParams,
    /// Regions the project's tracks were made for (manual boxes always
    /// apply).
    pub enabled: RegionSet,
    /// Whether an analysis has produced tracks.
    pub analyzed: bool,
    /// Verification state.
    pub verify: ProjectVerify,
    /// Timeline thumbnails, once an analysis made them.
    pub thumbs: Option<ThumbnailStrip>,
}

/// A span in which a region's detection fell back to its safe slot boxes
/// (`RegionHealth` warning).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct HealthSpan {
    /// The region.
    pub region: Region,
    /// First frame.
    pub f0: u32,
    /// Last frame (inclusive).
    pub f1: u32,
}

/// The tracks and edits of a project with the binding state of every edit
/// (`get_tracks`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct TrackSet {
    /// Project revision the set belongs to.
    pub revision: u64,
    /// Detector and OCR tracks.
    pub tracks: Vec<Track>,
    /// User edits, in list order.
    pub overrides: Vec<Override>,
    /// One binding per edit, in list order.
    pub bindings: Vec<OverrideBinding>,
    /// Spans where a region fell back to its safe slot boxes.
    pub health: Vec<HealthSpan>,
}

/// Presentation timestamps of every decoded frame (`get_pts`); the index
/// is the presentation frame index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct FramePts {
    /// Time base of the timestamps.
    pub time_base: Rational,
    /// One timestamp per frame, in `time_base` units.
    pub pts: Vec<i64>,
}

/// What `set_overrides` answers: the new revision and the frame ranges
/// edits changed since the last verification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct OverridesSaved {
    /// The project revision after the save.
    pub revision: u64,
    /// Ranges verified again before the next export.
    pub unverified_ranges: Vec<FrameRange>,
}

/// Where an export would go and what it would cost
/// (`get_output_preview`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct OutputPreview {
    /// The output file after the naming template and the conflict policy.
    pub path: PathBuf,
    /// Estimated size in bytes.
    pub est_size: u64,
    /// Estimated duration of the export in seconds.
    pub est_seconds: f64,
}

/// Detector options of `start_analysis` that are not part of a preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct DetectorOptions {
    /// Sensitivity.
    pub sensitivity: Sensitivity,
    /// Temporal parameters.
    pub temporal: TemporalParams,
}

/// One Exact patch of a frame: a region of interest blurred exactly like
/// the export, served as `blurrd://exact/<project>/<frame>/<roi>.jpg`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ExactPatchInfo {
    /// Name of the patch in its URL.
    pub roi: String,
    /// Where the patch goes, in source pixels.
    pub rect: RectI,
}

/// The export settings an Exact request blurs with: the `settings` query
/// of `blurrd://exact/...` URLs, as JSON. The patches then show what an
/// export of the file with these settings writes, whatever the cached
/// project holds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ExactSettings {
    /// The blur of the export.
    pub blur: BlurParams,
    /// The detector regions the export blurs (manual boxes always are).
    pub enabled: RegionSet,
}

/// What `blurrd://exact/<project>/<frame>.json` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ExactFrame {
    /// The presentation index the backend decoded.
    pub frame: u32,
    /// The project revision the patches were blurred with.
    pub revision: u64,
    /// The patches, which together cover every blurred box of the frame.
    pub patches: Vec<ExactPatchInfo>,
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{FramePts, ProbeOutcome};
    use crate::{AppError, ErrorKind, Rational};

    #[test]
    fn probe_outcomes_are_untagged() {
        let outcome = ProbeOutcome::Error {
            path: PathBuf::from("a.txt"),
            error: AppError::new(ErrorKind::UnsupportedMedia, "not a video"),
        };
        let json = serde_json::to_value(&outcome).unwrap();
        assert_eq!(json["path"], "a.txt");
        assert_eq!(json["error"]["kind"], "UnsupportedMedia");
        assert!(json.get("Error").is_none());
    }

    #[test]
    fn frame_pts_serialise_as_plain_numbers() {
        let pts = FramePts {
            time_base: Rational::new(1, 1000).unwrap(),
            pts: vec![0, 17, 33],
        };
        assert_eq!(
            serde_json::to_string(&pts).unwrap(),
            r#"{"time_base":[1,1000],"pts":[0,17,33]}"#
        );
    }
}
