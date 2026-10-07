//! Information about an input video, as reported by the probe.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::time::Rational;

/// Identifies a source file without hashing all of it.
///
/// Computed by the engine (size, modification time and a BLAKE3 hash of the
/// first and last megabytes); `blurrd-core` only defines the type.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Fingerprint {
    /// File size in bytes.
    pub size: u64,
    /// Modification time in seconds since the Unix epoch.
    pub mtime: i64,
    /// Hex BLAKE3 hash of the head and tail of the file.
    pub blake3_head_tail: String,
}

/// Whether video samples use the limited (TV) or full (PC) range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum ColorRange {
    /// Limited range (16 to 235 for 8-bit luma).
    Limited,
    /// Full range (0 to 255 for 8-bit luma).
    Full,
    /// Not tagged in the file.
    Unspecified,
}

/// Colour tags of a video stream, using FFmpeg's names (for example
/// `bt709`, `smpte2084`, `unknown`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ColorInfo {
    /// Colour primaries.
    pub primaries: String,
    /// Transfer characteristics.
    pub transfer: String,
    /// YUV matrix coefficients.
    pub matrix: String,
    /// Sample range.
    pub range: ColorRange,
}

/// High dynamic range transfer functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum HdrKind {
    /// Perceptual quantizer (SMPTE ST 2084), used by HDR10.
    Pq,
    /// Hybrid log-gamma (ARIB STD-B67).
    Hlg,
}

/// One audio stream of the input.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct AudioStreamInfo {
    /// Stream index in the container.
    pub index: u32,
    /// FFmpeg codec name, for example `aac` or `opus`.
    pub codec: String,
    /// Number of channels.
    pub channels: u16,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Bit rate in bits per second, if known.
    pub bitrate: Option<u64>,
    /// Language tag, if present.
    pub language: Option<String>,
}

/// Everything blurrd needs to know about an input video.
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent stream facts reported by the probe, not a state machine"
)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct MediaInfo {
    /// Path of the file.
    pub path: PathBuf,
    /// File size in bytes.
    pub size: u64,
    /// Duration in seconds.
    pub duration_s: f64,
    /// Width of the video in pixels.
    pub width: u32,
    /// Height of the video in pixels.
    pub height: u32,
    /// Average frame rate.
    pub fps: Rational,
    /// Time base of the video stream.
    pub time_base: Rational,
    /// Number of frames, if the container states it.
    pub frame_count: Option<u32>,
    /// FFmpeg codec name of the video stream, for example `h264` or `hevc`.
    pub video_codec: String,
    /// Codec profile, for example `High` or `Main 10`.
    pub profile: String,
    /// Bits per sample (8 or 10).
    pub bit_depth: u8,
    /// Colour tags.
    pub color: ColorInfo,
    /// HDR transfer function, if any.
    pub hdr: Option<HdrKind>,
    /// Whether the stream uses B-frames (decode order differs from
    /// presentation order).
    pub has_b_frames: bool,
    /// Whether the container has an edit list that shifts presentation.
    pub has_edit_list: bool,
    /// Presentation indices of the keyframes, if known.
    pub keyframes: Option<Vec<u32>>,
    /// Overall bit rate in bits per second, if known.
    pub bitrate: Option<u64>,
    /// Audio streams, copied through unchanged on export.
    pub audio: Vec<AudioStreamInfo>,
    /// Whether the file has subtitle streams.
    pub has_subtitles: bool,
    /// Whether the frame rate varies.
    pub is_vfr: bool,
}
