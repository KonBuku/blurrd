//! Detection, verification and application settings, and presets.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::blur_params::{BlurParams, Strength};
use crate::capabilities::Luid;
use crate::export::{EncoderSetting, ExportSettings, NvencSessions};
use crate::region::{Region, RegionSet};
use crate::schema::{Document, SchemaError};

/// How tightly boxes follow the names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Mode {
    /// Boxes hug the names; low-confidence spans fall back to the whole name
    /// slot automatically.
    #[default]
    Tight,
    /// Every name gets the union of its tight box and its slot box.
    Safe,
}

/// Detector sensitivity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum Sensitivity {
    /// Fewer false positives, may hold boxes less long.
    Low,
    /// The tuned default.
    #[default]
    Balanced,
    /// Catch more, accept more false positives.
    High,
}

/// Temporal parameters of the detector, in frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct TemporalParams {
    /// Frames a box starts before its first detection.
    pub pre_roll: u32,
    /// Minimum frames a box stays after its last detection.
    pub post_roll_min: u32,
    /// Longest gap between detections that is bridged.
    pub max_gap: u32,
}

impl Default for TemporalParams {
    fn default() -> Self {
        Self {
            pre_roll: 3,
            post_roll_min: 18,
            max_gap: 45,
        }
    }
}

/// Which regions are blurred, and how.
///
/// Fields of older revisions (kept names, chat options) are ignored when
/// a file is read, so presets and settings that hold them still load.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default)]
pub struct RegionSettings {
    /// Enabled detector regions.
    pub enabled: RegionSet,
    /// Box geometry mode (the engine may still choose
    /// [`Mode::Safe`] when the HUD calibration is unsure).
    pub mode: Mode,
}

impl Default for RegionSettings {
    fn default() -> Self {
        Self {
            enabled: RegionSet::all(),
            mode: Mode::Tight,
        }
    }
}

/// Which frames the verification reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Coverage {
    /// Every frame on which a HUD structure signal is on, plus one frame in
    /// fifteen elsewhere.
    #[default]
    Structure,
    /// Every frame.
    EveryFrame,
}

/// OCR verification settings.
///
/// Fields of older revisions (fix or report, full-frame interval,
/// confidence, output audit, ignored words) are ignored when a file is
/// read, so presets and settings that hold them still load.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default)]
pub struct VerifySettings {
    /// Run the verification during analysis.
    pub enabled: bool,
    /// Which frames are read.
    pub coverage: Coverage,
}

impl Default for VerifySettings {
    fn default() -> Self {
        Self {
            enabled: true,
            coverage: Coverage::Structure,
        }
    }
}

/// Colour theme of the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Theme {
    /// Follow Windows.
    System,
    /// Light.
    Light,
    /// Dark, the theme of a fresh install.
    #[default]
    Dark,
}

/// Whether animations are reduced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum ReduceMotion {
    /// Follow Windows.
    #[default]
    System,
    /// Reduce animations.
    On,
    /// Full animations.
    Off,
}

/// Appearance settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default)]
pub struct Appearance {
    /// Colour theme.
    pub theme: Theme,
    /// Base UI font size in pixels, 12 to 16.
    pub ui_font_size: u8,
    /// Animation preference.
    pub reduce_motion: ReduceMotion,
    /// Show thumbnails in the timeline.
    pub timeline_thumbs: bool,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme: Theme::Dark,
            ui_font_size: 14,
            reduce_motion: ReduceMotion::System,
            timeline_thumbs: true,
        }
    }
}

/// Hardware decoding preference.
///
/// The desktop app maps it to the engine's GPU switch when it starts:
/// `Software` decodes on the CPU and encodes with Media Foundation,
/// anything else lets blurrd pick the GPU decoder from its cached
/// benchmark (`Nvdec` cannot be forced yet and behaves like `Auto`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum HwDecodeSetting {
    /// Choose per codec from a cached benchmark.
    #[default]
    Auto,
    /// Prefer NVDEC.
    Nvdec,
    /// Always decode on the CPU.
    Software,
}

/// Resolution of the in-app preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum PreviewQuality {
    /// Full resolution when the machine keeps up, otherwise reduced.
    #[default]
    Auto,
    /// Always full resolution.
    Full,
    /// Always the reduced proxy resolution.
    Reduced,
}

/// Processing settings.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default)]
pub struct Processing {
    /// GPU to use, or the high-performance adapter if `None`.
    pub gpu_luid: Option<Luid>,
    /// Decoder preference.
    pub hw_decode: HwDecodeSetting,
    /// Encoder preference.
    pub encoder: EncoderSetting,
    /// NVENC sessions per export.
    pub nvenc_sessions: NvencSessions,
    /// Jobs that run at the same time, 1 or 2.
    pub parallel_jobs: u8,
    /// Preview resolution.
    pub preview_quality: PreviewQuality,
}

impl Processing {
    /// Why these processing settings cannot be used, or `None`.
    pub fn problem(&self) -> Option<String> {
        (!(1..=2).contains(&self.parallel_jobs))
            .then(|| format!("parallel jobs {} must be 1 or 2", self.parallel_jobs))
    }

    /// Whether the GPU may decode and encode (everything but `Software`).
    pub fn uses_gpu(&self) -> bool {
        self.hw_decode != HwDecodeSetting::Software
    }
}

impl Default for Processing {
    fn default() -> Self {
        Self {
            gpu_luid: None,
            hw_decode: HwDecodeSetting::Auto,
            encoder: EncoderSetting::Auto,
            nvenc_sessions: NvencSessions::Auto,
            parallel_jobs: 1,
            preview_quality: PreviewQuality::Auto,
        }
    }
}

/// Defaults for new projects.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default)]
pub struct Defaults {
    /// Region settings.
    pub regions: RegionSettings,
    /// Blur parameters.
    pub blur: BlurParams,
    /// Export settings.
    pub export: ExportSettings,
    /// Verification settings.
    pub verify: VerifySettings,
}

impl Defaults {
    /// Why these defaults cannot be used, or `None`: invalid blur or
    /// export values.
    pub fn problem(&self) -> Option<String> {
        validate_defaults(&self.blur, &self.export)
            .err()
            .map(|error| match error {
                SchemaError::Invalid(problem) => problem,
                other => other.to_string(),
            })
    }
}

/// General behaviour.
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent on/off preferences"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default)]
pub struct General {
    /// Restore the queue after a restart.
    pub restore_queue: bool,
    /// Ask before removing a running job from the queue.
    pub confirm_remove_running: bool,
    /// Open the output folder after an export.
    pub open_folder_after_export: bool,
    /// Show a notification when a job finishes.
    pub notify: bool,
}

impl Default for General {
    fn default() -> Self {
        Self {
            restore_queue: true,
            confirm_remove_running: true,
            open_folder_after_export: false,
            notify: true,
        }
    }
}

/// Log verbosity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum LogLevel {
    /// Errors only.
    Error,
    /// Errors and warnings.
    Warn,
    /// Normal operation (the default). Logs never contain OCR text.
    #[default]
    Info,
    /// Details for bug reports.
    Debug,
    /// Everything.
    Trace,
}

/// Privacy settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default)]
pub struct Privacy {
    /// Log verbosity.
    pub log_level: LogLevel,
}

/// The app settings file (`settings.json`, schema `blurrd-settings/1`).
///
/// Missing fields take their defaults, so a settings file from an older
/// revision of the same major loads unchanged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Settings {
    /// Always `blurrd-settings/1`.
    pub schema: String,
    /// Revision within the schema's major version.
    pub version: u32,
    /// Appearance.
    #[serde(default)]
    pub appearance: Appearance,
    /// Processing.
    #[serde(default)]
    pub processing: Processing,
    /// Defaults for new projects.
    #[serde(default)]
    pub defaults: Defaults,
    /// General behaviour.
    #[serde(default)]
    pub general: General,
    /// Privacy.
    #[serde(default)]
    pub privacy: Privacy,
    /// Keyboard shortcuts that differ from the defaults: action name to key.
    #[serde(default)]
    pub shortcuts: BTreeMap<String, String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema: Self::schema_id(),
            version: Self::REVISION,
            appearance: Appearance::default(),
            processing: Processing::default(),
            defaults: Defaults::default(),
            general: General::default(),
            privacy: Privacy::default(),
            shortcuts: BTreeMap::new(),
        }
    }
}

impl Document for Settings {
    const NAME: &'static str = "blurrd-settings";
    const MAJOR: u32 = 1;
    const REVISION: u32 = 1;

    fn validate(&self) -> Result<(), SchemaError> {
        if !(12..=16).contains(&self.appearance.ui_font_size) {
            return Err(SchemaError::Invalid(format!(
                "UI font size {} is outside 12..=16",
                self.appearance.ui_font_size
            )));
        }
        if let Some(problem) = self.processing.problem() {
            return Err(SchemaError::Invalid(problem));
        }
        validate_defaults(&self.defaults.blur, &self.defaults.export)
    }
}

/// A named set of region, blur, export and verification settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Preset {
    /// Stable identifier.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Whether the preset ships with blurrd (read-only).
    pub builtin: bool,
    /// Region settings.
    pub regions: RegionSettings,
    /// Blur parameters.
    pub blur: BlurParams,
    /// Export settings.
    pub export: ExportSettings,
    /// Verification settings.
    pub verify: VerifySettings,
}

impl Preset {
    /// The presets that ship with blurrd: *Default*, *Max privacy* and
    /// *Kill feed only*.
    pub fn builtins() -> Vec<Self> {
        let default = Self {
            id: "default".to_owned(),
            name: "Default".to_owned(),
            builtin: true,
            regions: RegionSettings::default(),
            blur: BlurParams::default(),
            export: ExportSettings::default(),
            verify: VerifySettings::default(),
        };
        let max_privacy = Self {
            id: "max-privacy".to_owned(),
            name: "Max privacy".to_owned(),
            regions: RegionSettings {
                mode: Mode::Safe,
                ..RegionSettings::default()
            },
            blur: BlurParams {
                strength: Strength::High,
                ..BlurParams::default()
            },
            verify: VerifySettings {
                coverage: Coverage::EveryFrame,
                ..VerifySettings::default()
            },
            ..default.clone()
        };
        let kill_feed_only = Self {
            id: "kill-feed-only".to_owned(),
            name: "Kill feed only".to_owned(),
            regions: RegionSettings {
                enabled: RegionSet::empty().with(Region::KillFeed),
                ..RegionSettings::default()
            },
            ..default.clone()
        };
        vec![default, max_privacy, kill_feed_only]
    }
}

/// A presets file (`presets.json`, or an exported `.blurrd-preset.json`),
/// schema `blurrd-presets/1`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PresetFile {
    /// Always `blurrd-presets/1`.
    pub schema: String,
    /// Revision within the schema's major version.
    pub version: u32,
    /// The presets.
    pub presets: Vec<Preset>,
}

impl PresetFile {
    /// A presets file holding `presets`.
    pub fn new(presets: Vec<Preset>) -> Self {
        Self {
            schema: Self::schema_id(),
            version: Self::REVISION,
            presets,
        }
    }
}

impl Document for PresetFile {
    const NAME: &'static str = "blurrd-presets";
    const MAJOR: u32 = 1;
    const REVISION: u32 = 1;

    fn validate(&self) -> Result<(), SchemaError> {
        let mut ids: Vec<&str> = self.presets.iter().map(|p| p.id.as_str()).collect();
        ids.sort_unstable();
        if let Some(id) = ids.windows(2).find(|w| w[0] == w[1]).map(|w| w[0]) {
            return Err(SchemaError::Invalid(format!(
                "preset id `{id}` is used twice"
            )));
        }
        self.presets
            .iter()
            .try_for_each(|p| validate_defaults(&p.blur, &p.export))
    }
}

/// Checks the blur and export part of settings and presets.
fn validate_defaults(blur: &BlurParams, export: &ExportSettings) -> Result<(), SchemaError> {
    if !blur.is_valid() {
        return Err(SchemaError::Invalid(format!(
            "blur feather {} is invalid",
            blur.feather
        )));
    }
    if let Some(problem) = export.problem() {
        return Err(SchemaError::Invalid(problem));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        Coverage, Defaults, HwDecodeSetting, Mode, Preset, PresetFile, Processing, Settings,
    };
    use crate::region::Region;
    use crate::schema::{self, SchemaError};

    #[test]
    fn default_settings_round_trip() {
        let json = schema::to_json(&Settings::default()).unwrap();
        assert!(json.starts_with(r#"{"schema":"blurrd-settings/1","version":1,"#));
        assert_eq!(
            schema::from_json::<Settings>(&json).unwrap(),
            Settings::default()
        );
    }

    #[test]
    fn missing_settings_take_defaults() {
        let settings: Settings = schema::from_json(
            r#"{ "schema": "blurrd-settings/1", "version": 1,
                 "appearance": { "theme": "Dark" } }"#,
        )
        .unwrap();
        assert_eq!(settings.appearance.ui_font_size, 14);
        assert_eq!(settings.defaults, Settings::default().defaults);
    }

    #[test]
    fn rejects_out_of_range_settings() {
        let settings = r#"{ "schema": "blurrd-settings/1", "version": 1,
                            "appearance": { "ui_font_size": 30 } }"#;
        assert!(matches!(
            schema::from_json::<Settings>(settings),
            Err(SchemaError::Invalid(_))
        ));
    }

    #[test]
    fn sections_report_their_problems() {
        assert_eq!(Processing::default().problem(), None);
        let processing = Processing {
            parallel_jobs: 0,
            ..Processing::default()
        };
        assert!(processing.problem().is_some());
        assert!(Processing::default().uses_gpu());
        let software = Processing {
            hw_decode: HwDecodeSetting::Software,
            ..Processing::default()
        };
        assert!(!software.uses_gpu());

        assert_eq!(Defaults::default().problem(), None);
        let mut defaults = Defaults::default();
        defaults.export.output.template = " ".to_owned();
        assert!(defaults.problem().is_some());
    }

    #[test]
    fn builtin_presets_match_the_plan() {
        let presets = Preset::builtins();
        let ids: Vec<&str> = presets.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["default", "max-privacy", "kill-feed-only"]);
        assert_eq!(presets[1].regions.mode, Mode::Safe);
        assert_eq!(
            presets[2].regions.enabled.iter().collect::<Vec<_>>(),
            [Region::KillFeed]
        );
        let file = PresetFile::new(presets);
        let json = schema::to_json_pretty(&file).unwrap();
        assert_eq!(schema::from_json::<PresetFile>(&json).unwrap(), file);
    }

    #[test]
    fn presets_of_older_revisions_still_load() {
        // Kept names, chat options and the verification options of earlier
        // revisions were never read by the engine; they are ignored now.
        let mut preset = Preset::builtins().remove(0);
        preset.id = "user-1".to_owned();
        preset.builtin = false;
        let mut json = serde_json::to_value(PresetFile::new(vec![preset])).unwrap();
        let old = &mut json["presets"][0];
        old["regions"] = serde_json::json!({
            "enabled": ["KillFeed"], "mode": "Safe",
            "keep_names": { "own_row": true, "names": ["someone"] },
            "chat_blur_message": true, "chat_blur_system": false });
        old["verify"] = serde_json::json!({
            "enabled": true, "mode": "ReportOnly", "coverage": "EveryFrame",
            "full_frame_sample_ms": 500, "min_conf": 0.7,
            "audit_output": "Sampled", "ignore_words": ["ACE"] });
        let json = json.to_string();
        let file: PresetFile = schema::from_json(&json).unwrap();
        let preset = &file.presets[0];
        assert_eq!(preset.regions.mode, Mode::Safe);
        assert_eq!(
            preset.regions.enabled.iter().collect::<Vec<_>>(),
            [Region::KillFeed]
        );
        assert_eq!(preset.verify.coverage, Coverage::EveryFrame);
        let again = schema::to_json(&file).unwrap();
        assert!(!again.contains("keep_names") && !again.contains("min_conf"));
    }

    #[test]
    fn rejects_duplicate_preset_ids() {
        let mut presets = Preset::builtins();
        presets[1].id = "default".to_owned();
        assert!(matches!(
            schema::to_json(&PresetFile::new(presets)),
            Err(SchemaError::Invalid(_))
        ));
    }
}
