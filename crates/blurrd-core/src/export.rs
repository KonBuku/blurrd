//! Export settings (also the payload of presets).
//!
//! Resolution and frame rate always match the source; these settings choose
//! codec, encoder, quality, streams and the output file.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::time::FrameRange;

/// Output video codec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum Codec {
    /// The source's codec (H.264, HEVC or AV1).
    #[default]
    Source,
    /// H.264 / AVC.
    H264,
    /// H.265 / HEVC.
    Hevc,
    /// AV1.
    Av1,
}

/// Which encoder to use.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum EncoderSetting {
    /// The fallback chain NVENC, AMF, Quick Sync, Media Foundation.
    #[default]
    Auto,
    /// A specific FFmpeg encoder, for example `h264_nvenc`.
    Named(String),
}

/// Rate control of the encoder.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum RateControl {
    /// Variable bit rate around the source's bit rate, capped at
    /// `max_factor` times the source.
    MatchSource {
        /// Peak bit rate as a multiple of the source bit rate.
        max_factor: f32,
    },
    /// Constant quality.
    Cq(u8),
    /// Variable bit rate with explicit targets.
    Vbr {
        /// Target bit rate in kbit/s.
        target_kbps: u32,
        /// Peak bit rate in kbit/s.
        max_kbps: u32,
    },
}

impl Default for RateControl {
    fn default() -> Self {
        Self::MatchSource { max_factor: 1.5 }
    }
}

/// An on/off setting that can also follow the source or the hardware.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Toggle {
    /// Decide from the source and the encoder's capabilities.
    #[default]
    Auto,
    /// Always on.
    On,
    /// Always off.
    Off,
}

/// Number of parallel NVENC sessions within one export.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum NvencSessions {
    /// Two sessions for H.264 when the GPU supports them, otherwise one.
    #[default]
    Auto,
    /// One session.
    One,
    /// Two sessions.
    Two,
}

/// Where the output's keyframes go.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum KeyframePolicy {
    /// An IDR frame wherever the source has a keyframe, closed GOPs.
    #[default]
    MatchSource,
    /// The encoder's own keyframe placement.
    EncoderDefault,
}

/// What happens to the audio streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum AudioMode {
    /// Copy every audio stream unchanged.
    #[default]
    Copy,
    /// Re-encode to AAC at the given bit rate in kbit/s.
    Aac(u16),
    /// Drop audio.
    Remove,
}

/// Which container metadata is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum MetadataMode {
    /// Only colour, rotation and timecode; recorder title, comment and
    /// account tags are stripped.
    #[default]
    Technical,
    /// Everything the source has.
    All,
}

/// Output container format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum Container {
    /// The source's container.
    #[default]
    Source,
    /// MP4.
    Mp4,
    /// Matroska.
    Mkv,
    /// QuickTime.
    Mov,
}

/// Where the MP4/MOV index (`moov`) is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum MoovMode {
    /// Reserve space up front; fall back to a faststart rewrite if it is
    /// too small.
    #[default]
    Reserve,
    /// Rewrite the file to move the index to the front.
    Faststart,
    /// Write the index at the end.
    AtEnd,
}

/// Where output files go.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum OutputFolder {
    /// The source file's folder.
    #[default]
    NextToSource,
    /// A fixed folder.
    Folder(PathBuf),
}

/// What to do when the output file already exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum ConflictPolicy {
    /// Append a number: `clip_blurrd (2).mp4`.
    #[default]
    AddNumber,
    /// Replace the existing file (never the input).
    Overwrite,
    /// Skip the export.
    Skip,
}

/// Output file naming.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct OutputSettings {
    /// Output folder.
    pub folder: OutputFolder,
    /// File name template without extension; `{name}` is the source's file
    /// stem.
    pub template: String,
    /// Behaviour when the file exists.
    pub on_conflict: ConflictPolicy,
}

impl Default for OutputSettings {
    fn default() -> Self {
        Self {
            folder: OutputFolder::NextToSource,
            template: "{name}_blurrd".to_owned(),
            on_conflict: ConflictPolicy::AddNumber,
        }
    }
}

/// Export settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ExportSettings {
    /// Output codec.
    pub codec: Codec,
    /// Encoder choice.
    pub encoder: EncoderSetting,
    /// Rate control.
    pub rate: RateControl,
    /// Encoder preset, 1 (fastest) to 7 (best quality).
    pub preset: u8,
    /// 10-bit output.
    pub ten_bit: Toggle,
    /// Split-frame encoding for HEVC and AV1.
    pub split_encode: Toggle,
    /// Parallel NVENC sessions.
    pub nvenc_sessions: NvencSessions,
    /// Keyframe placement.
    pub keyframes: KeyframePolicy,
    /// Stream-copy GOPs without boxes (not available in v0.1).
    pub smart_render: bool,
    /// Audio handling.
    pub audio: AudioMode,
    /// Copy subtitle streams.
    pub copy_subtitles: bool,
    /// Container metadata.
    pub metadata: MetadataMode,
    /// Output container.
    pub container: Container,
    /// Index placement for MP4/MOV.
    pub moov: MoovMode,
    /// Export only these frames, or the whole video.
    pub trim: Option<FrameRange>,
    /// Output file naming.
    pub output: OutputSettings,
}

/// Lowest encoder preset.
pub const MIN_PRESET: u8 = 1;
/// Highest encoder preset.
pub const MAX_PRESET: u8 = 7;

impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            codec: Codec::Source,
            encoder: EncoderSetting::Auto,
            rate: RateControl::default(),
            preset: 3,
            ten_bit: Toggle::Auto,
            split_encode: Toggle::Auto,
            nvenc_sessions: NvencSessions::Auto,
            keyframes: KeyframePolicy::MatchSource,
            smart_render: false,
            audio: AudioMode::Copy,
            copy_subtitles: true,
            metadata: MetadataMode::Technical,
            container: Container::Source,
            moov: MoovMode::Reserve,
            trim: None,
            output: OutputSettings::default(),
        }
    }
}

impl ExportSettings {
    /// Describes the first invalid value, if any: a preset outside
    /// `1..=7`, a non-positive or non-finite bit rate factor, a VBR peak
    /// below its target, or an empty file name template.
    pub fn problem(&self) -> Option<String> {
        if !(MIN_PRESET..=MAX_PRESET).contains(&self.preset) {
            return Some(format!(
                "preset {} is outside {MIN_PRESET}..={MAX_PRESET}",
                self.preset
            ));
        }
        match self.rate {
            RateControl::MatchSource { max_factor }
                if !max_factor.is_finite() || max_factor <= 0.0 =>
            {
                return Some(format!("bit rate factor {max_factor} must be positive"));
            }
            RateControl::Vbr {
                target_kbps,
                max_kbps,
            } if max_kbps < target_kbps => {
                return Some(format!(
                    "peak bit rate {max_kbps} is below the target {target_kbps}"
                ));
            }
            _ => {}
        }
        self.output
            .template
            .trim()
            .is_empty()
            .then(|| "the file name template is empty".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::{ExportSettings, RateControl};

    #[test]
    fn default_matches_the_documented_format() {
        let expected = serde_json::json!({
            "codec": "Source", "encoder": "Auto", "rate": { "MatchSource": { "max_factor": 1.5 } },
            "preset": 3, "ten_bit": "Auto", "split_encode": "Auto", "nvenc_sessions": "Auto",
            "keyframes": "MatchSource", "smart_render": false,
            "audio": "Copy", "copy_subtitles": true, "metadata": "Technical",
            "container": "Source", "moov": "Reserve", "trim": null,
            "output": { "folder": "NextToSource", "template": "{name}_blurrd", "on_conflict": "AddNumber" }
        });
        assert_eq!(
            serde_json::to_value(ExportSettings::default()).unwrap(),
            expected
        );
        let back: ExportSettings = serde_json::from_value(expected).unwrap();
        assert_eq!(back, ExportSettings::default());
    }

    #[test]
    fn reads_other_rate_controls() {
        let cq: RateControl = serde_json::from_str(r#"{ "Cq": 19 }"#).unwrap();
        assert_eq!(cq, RateControl::Cq(19));
        let vbr: RateControl =
            serde_json::from_str(r#"{ "Vbr": { "target_kbps": 30000, "max_kbps": 45000 } }"#)
                .unwrap();
        assert_eq!(
            vbr,
            RateControl::Vbr {
                target_kbps: 30000,
                max_kbps: 45000
            }
        );
    }

    #[test]
    fn reports_invalid_values() {
        assert_eq!(ExportSettings::default().problem(), None);
        let bad_preset = ExportSettings {
            preset: 9,
            ..ExportSettings::default()
        };
        assert!(bad_preset.problem().is_some());
        let bad_vbr = ExportSettings {
            rate: RateControl::Vbr {
                target_kbps: 10,
                max_kbps: 5,
            },
            ..ExportSettings::default()
        };
        assert!(bad_vbr.problem().is_some());
    }
}
