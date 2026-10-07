//! What the machine can do: GPUs, decoders, encoders and OCR.

use std::fmt;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A Windows adapter LUID (locally unique identifier) of a GPU.
///
/// Adapters are always chosen by LUID, never by index. Written as 16 hex
/// digits (`"00000000000a1b2c"`) because JavaScript numbers cannot hold every
/// 64-bit value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(try_from = "String", into = "String")]
#[ts(as = "String")]
pub struct Luid(pub u64);

/// The error returned for a LUID string that is not 16 hex digits.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{0}` is not a LUID (16 hexadecimal digits)")]
pub struct InvalidLuidError(pub String);

impl TryFrom<String> for Luid {
    type Error = InvalidLuidError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        if text.len() != 16 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(InvalidLuidError(text));
        }
        u64::from_str_radix(&text, 16)
            .map(Self)
            .map_err(|_| InvalidLuidError(text))
    }
}

impl From<Luid> for String {
    fn from(luid: Luid) -> Self {
        luid.to_string()
    }
}

impl fmt::Display for Luid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

/// A GPU, as reported by DXGI.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct GpuInfo {
    /// Adapter LUID.
    pub luid: Luid,
    /// Adapter name.
    pub name: String,
    /// Dedicated video memory in bytes.
    pub vram: u64,
    /// Driver version.
    pub driver: String,
}

/// A video decoder available to the engine.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct DecoderInfo {
    /// FFmpeg codec name, for example `hevc`.
    pub codec: String,
    /// Decoder or hardware method, for example `nvdec` or `software`.
    pub name: String,
    /// Whether it decodes on the GPU.
    pub hardware: bool,
    /// Whether it decodes 10-bit streams.
    pub ten_bit: bool,
}

/// A video encoder and whether it works on this machine.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct EncoderInfo {
    /// FFmpeg codec name of its output, for example `h264`.
    pub codec: String,
    /// FFmpeg encoder name, for example `h264_nvenc`.
    pub name: String,
    /// Whether it encodes on the GPU.
    pub hardware: bool,
    /// Whether it encodes 10-bit output.
    pub ten_bit: bool,
    /// Whether a test encode succeeded.
    pub available: bool,
}

/// Progress of the background encoder probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum ProbeState {
    /// Still probing; decoder and encoder lists may be incomplete.
    Pending,
    /// Probing finished.
    Ready,
}

/// ONNX Runtime execution providers used for OCR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum OcrProvider {
    /// DirectML on a GPU.
    DirectMl,
    /// TensorRT for RTX on an NVIDIA GPU.
    TensorRtRtx,
    /// The CPU.
    Cpu,
}

/// Whether OCR verification can run.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum OcrCapability {
    /// Not checked yet; OCR is loaded on first use.
    NotProbed,
    /// OCR runs with this provider.
    Available {
        /// The active execution provider.
        provider: OcrProvider,
    },
    /// OCR cannot run; blurring still works without verification.
    Unavailable {
        /// Why, for example a missing runtime or model file.
        reason: String,
    },
}

/// Everything the UI shows about the machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Capabilities {
    /// GPUs, high-performance adapter first.
    pub gpus: Vec<GpuInfo>,
    /// Decoders.
    pub decoders: Vec<DecoderInfo>,
    /// Encoders.
    pub encoders: Vec<EncoderInfo>,
    /// Whether the background probe has finished.
    pub probe_state: ProbeState,
    /// OCR state.
    pub ocr: OcrCapability,
    /// Version of the linked FFmpeg libraries.
    pub ffmpeg_version: String,
    /// blurrd version.
    pub app_version: String,
}

#[cfg(test)]
mod tests {
    use super::Luid;

    #[test]
    fn luid_round_trips_as_hex() {
        let luid = Luid(0x0001_0000_00a1_b2c3);
        let json = serde_json::to_string(&luid).unwrap();
        assert_eq!(json, r#""0001000000a1b2c3""#);
        assert_eq!(serde_json::from_str::<Luid>(&json).unwrap(), luid);
        assert!(serde_json::from_str::<Luid>(r#""12345""#).is_err());
        assert!(serde_json::from_str::<Luid>(r#""+000000000000001""#).is_err());
    }
}
