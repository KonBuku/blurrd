//! The project file: everything known about one video after analysis.
//!
//! Stored as `project.json` in the app cache (one folder per source
//! fingerprint) or exported as `.blurrd.json`. Per-frame detector data is
//! never persisted; tracks and overrides are enough to render.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::blur_params::BlurParams;
use crate::hud::HudGeometry;
use crate::media::Fingerprint;
use crate::overrides::Override;
use crate::plan::{BlurPlan, Track};
use crate::region::RegionSet;
use crate::schema::{Document, SchemaError};
use crate::settings::{Mode, Sensitivity, TemporalParams};
use crate::time::{FrameRange, Rational};
use crate::verify::VerifyState;

/// The analysed video, as recorded in the project.
///
/// The PTS and keyframe hashes let the render detect a changed file
/// (`ProjectMismatch`) before it writes anything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ProjectMedia {
    /// Path of the source file.
    pub path: PathBuf,
    /// Fingerprint of the source file.
    pub fingerprint: Fingerprint,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Average frame rate.
    pub fps: Rational,
    /// Time base of the video stream.
    pub time_base: Rational,
    /// Number of decoded frames.
    pub frame_count: u32,
    /// Hex SHA-256 of the presentation timestamps (`pts.bin`).
    pub pts_sha256: String,
    /// Whether the frame rate varies.
    pub is_vfr: bool,
    /// Whether the stream uses B-frames.
    pub has_b_frames: bool,
    /// Hex SHA-256 of the keyframe list.
    pub keyframes_sha256: String,
}

/// How the tracks were produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct DetectorInfo {
    /// Detector version, for example `detect/1.0.0`.
    pub version: String,
    /// Regions that were analysed.
    pub regions: RegionSet,
    /// Box geometry mode.
    pub mode: Mode,
    /// Sensitivity.
    pub sensitivity: Sensitivity,
    /// Temporal parameters.
    pub temporal: TemporalParams,
}

/// Verification state of the project.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ProjectVerify {
    /// Project revision the last verification checked.
    pub plan_revision: u64,
    /// Result of the last verification.
    pub state: VerifyState,
    /// Frame ranges changed by edits since then; verified again before
    /// export.
    pub unverified_ranges: Vec<FrameRange>,
    /// File name of the last report in the project folder.
    pub report: Option<String>,
}

/// A project file (schema `blurrd-project/1`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Project {
    /// Always `blurrd-project/1`.
    pub schema: String,
    /// Revision within the schema's major version.
    pub version: u32,
    /// blurrd version that wrote the file.
    pub app_version: String,
    /// The analysed video.
    pub media: ProjectMedia,
    /// HUD geometry used by the detector.
    pub geometry: HudGeometry,
    /// How the tracks were produced.
    pub detector: DetectorInfo,
    /// Detector and OCR tracks. A re-analysis replaces them.
    pub tracks: Vec<Track>,
    /// User edits. A re-analysis keeps them; they rebind by overlap.
    pub overrides: Vec<Override>,
    /// Incremented on every change of tracks or overrides.
    pub revision: u64,
    /// Blur parameters.
    pub blur: BlurParams,
    /// Verification state.
    pub verify: ProjectVerify,
}

impl Project {
    /// A project without tracks or overrides at revision 0, not verified.
    pub fn new(
        media: ProjectMedia,
        geometry: HudGeometry,
        detector: DetectorInfo,
        blur: BlurParams,
    ) -> Self {
        Self {
            schema: Self::schema_id(),
            version: Self::REVISION,
            app_version: crate::VERSION.to_owned(),
            media,
            geometry,
            detector,
            tracks: Vec::new(),
            overrides: Vec::new(),
            revision: 0,
            blur,
            verify: ProjectVerify::default(),
        }
    }

    /// The blur plan of the project (a copy of its tracks and overrides).
    pub fn plan(&self) -> BlurPlan {
        BlurPlan {
            frame_count: self.media.frame_count,
            tracks: self.tracks.clone(),
            overrides: self.overrides.clone(),
        }
    }
}

impl Document for Project {
    const NAME: &'static str = "blurrd-project";
    const MAJOR: u32 = 1;
    const REVISION: u32 = 1;

    fn validate(&self) -> Result<(), SchemaError> {
        let invalid = |message: String| Err(SchemaError::Invalid(message));
        let media = &self.media;
        if media.width == 0 || media.height == 0 {
            return invalid(format!(
                "frame size {}x{} is empty",
                media.width, media.height
            ));
        }
        self.plan()
            .validate(media.width, media.height)
            .map_err(|e| SchemaError::Invalid(e.to_string()))?;
        if !self.blur.is_valid() {
            return invalid(format!("blur feather {} is invalid", self.blur.feather));
        }
        if self.verify.plan_revision > self.revision {
            return invalid(format!(
                "verified revision {} is newer than the project revision {}",
                self.verify.plan_revision, self.revision
            ));
        }
        if let Some(range) = self
            .verify
            .unverified_ranges
            .iter()
            .find(|r| r.f1 >= media.frame_count)
        {
            return invalid(format!(
                "unverified range {}..={} reaches past the last frame",
                range.f0, range.f1
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Project;
    use crate::region::{Region, RegionSet};
    use crate::schema::{self, SchemaError};
    use crate::verify::VerifyState;

    /// The example from section 6.1 of the architecture plan, with the
    /// elided hashes and ids filled in.
    const EXAMPLE: &str = r#"{
      "schema": "blurrd-project/1",
      "version": 1,
      "app_version": "0.1.0",
      "media": {
        "path": "D:/clips/match.mp4",
        "fingerprint": { "size": 214748364, "mtime": 1728300000,
                         "blake3_head_tail": "9f1c0000000000000000000000000000000000000000000000000000000000aa" },
        "width": 1920, "height": 1080, "fps": [60, 1], "time_base": [1, 15360],
        "frame_count": 3601,
        "pts_sha256": "4b2e000000000000000000000000000000000000000000000000000000000000",
        "is_vfr": false, "has_b_frames": false,
        "keyframes_sha256": "77aa000000000000000000000000000000000000000000000000000000000000"
      },
      "geometry": { "layout": "Auto", "sx": 1.0, "sy": 1.0, "anchor_left": 0, "anchor_right": 1920,
                    "anchor_center": 960, "oy": 0, "confidence": 0.98,
                    "per_region": { "KillFeed": 0.99, "Banner": 0.97, "VoiceChat": 0.95, "TextChat": 0.96 } },
      "detector": { "version": "detect/1.0.0", "regions": ["KillFeed","Banner","VoiceChat","TextChat",
                                                           "Scoreboard","DeathRecap","Spectating","MatchEnd"],
                    "mode": "Tight", "sensitivity": "Balanced",
                    "temporal": { "pre_roll": 3, "post_roll_min": 18, "max_gap": 45 } },
      "tracks": [
        { "id": 17, "region": "KillFeed", "kind": "Victim", "source": "Detector",
          "segments": [ { "f0": 2611, "f1": 2649, "rect": [1782, 140, 1835, 163], "conf": "Tracked", "text_offset": [0, 0] },
                        { "f0": 2650, "f1": 2700, "rect": [1782, 101, 1835, 124], "conf": "Tracked", "text_offset": [0, -39] } ] }
      ],
      "overrides": [
        { "op": "Add", "uid": "0192f4a8-7c00-7000-8000-000000000001", "region": "Manual", "f0": 300, "f1": 420, "rect": [40, 40, 200, 70] },
        { "op": "Suppress", "uid": "0192f4a8-7c00-7000-8000-000000000002", "region": "KillFeed", "f0": 900, "f1": 910, "anchor": [1700, 101, 1760, 124] },
        { "op": "Adjust", "uid": "0192f4a8-7c00-7000-8000-000000000003", "region": "KillFeed", "f0": 2611, "f1": 2620,
          "anchor": [1782, 140, 1835, 163], "rect": [1778, 140, 1835, 163] }
      ],
      "revision": 12,
      "blur": { "style": "PixelBlur", "strength": "Medium", "pad": [4, 3], "radius": 6, "feather": 1.5, "noise": true },
      "verify": { "plan_revision": 12, "state": "Clean", "unverified_ranges": [],
                  "report": "verify-2026-10-07T20-31-11.json" }
    }"#;

    #[test]
    fn reads_the_documented_example() {
        let project: Project = schema::from_json(EXAMPLE).unwrap();
        assert_eq!(project.media.frame_count, 3601);
        assert_eq!(project.detector.regions, RegionSet::all());
        assert_eq!(project.verify.state, VerifyState::Clean);
        assert_eq!(project.geometry.per_region.0[&Region::Banner], 0.97);
        let plan = project.plan();
        assert_eq!(plan.overrides.len(), 3);
        let mut boxes = Vec::new();
        plan.boxes_at(2615, &RegionSet::all(), &mut boxes);
        assert_eq!(boxes[0].rect.x0, 1778, "the Adjust override applies");
    }

    #[test]
    fn round_trips() {
        let project: Project = schema::from_json(EXAMPLE).unwrap();
        let json = schema::to_json(&project).unwrap();
        assert_eq!(schema::from_json::<Project>(&json).unwrap(), project);
        let pretty = schema::to_json_pretty(&project).unwrap();
        assert_eq!(schema::from_json::<Project>(&pretty).unwrap(), project);
    }

    #[test]
    fn refuses_other_documents_and_newer_versions() {
        let newer = EXAMPLE.replace("blurrd-project/1", "blurrd-project/2");
        assert!(matches!(
            schema::from_json::<Project>(&newer),
            Err(SchemaError::NewerMajor {
                found: 2,
                supported: 1,
                ..
            })
        ));
        let settings = EXAMPLE.replace("blurrd-project/1", "blurrd-settings/1");
        assert!(matches!(
            schema::from_json::<Project>(&settings),
            Err(SchemaError::WrongDocument { .. })
        ));
        assert!(matches!(
            schema::from_json::<Project>(r#"{"tracks": []}"#),
            Err(SchemaError::WrongDocument { .. })
        ));
        assert!(matches!(
            schema::from_json::<Project>("{"),
            Err(SchemaError::Malformed(_))
        ));
    }

    #[test]
    fn ignores_unknown_fields_of_newer_revisions() {
        let newer_revision =
            EXAMPLE.replace(r#""version": 1,"#, r#""version": 7, "future": true,"#);
        assert!(schema::from_json::<Project>(&newer_revision).is_ok());
    }

    #[test]
    fn rejects_invalid_plans() {
        let outside = EXAMPLE.replace(
            "[1782, 140, 1835, 163], \"conf\"",
            "[1782, 140, 1935, 163], \"conf\"",
        );
        assert!(matches!(
            schema::from_json::<Project>(&outside),
            Err(SchemaError::Invalid(_))
        ));
        let stale = EXAMPLE.replace(r#""plan_revision": 12"#, r#""plan_revision": 13"#);
        assert!(matches!(
            schema::from_json::<Project>(&stale),
            Err(SchemaError::Invalid(_))
        ));
    }
}
