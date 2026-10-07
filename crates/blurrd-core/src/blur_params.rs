//! How boxes are blurred.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Smallest block size, in reference pixels (scaled by the HUD's `sy`).
///
/// Smaller blocks can be de-pixelated or reconstructed from several frames
/// (C17 in the architecture plan), so every strength is clamped to this.
pub const MIN_BLOCK_PX: u8 = 8;

/// Look of the blur inside a box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum BlurStyle {
    /// Pixelate, then smooth with a gaussian (the default).
    #[default]
    PixelBlur,
    /// Gaussian blur on the downsampled box.
    Gaussian,
    /// Hard pixel blocks.
    Pixelate,
    /// A solid fill.
    Solid {
        /// The fill colour.
        color: SolidColor,
    },
}

/// Fill colour of [`BlurStyle::Solid`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum SolidColor {
    /// The mean colour of the box.
    BoxMean,
    /// A fixed sRGB colour `[r, g, b]`.
    Rgb([u8; 3]),
}

/// Blur strength, as a block size in reference pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum Strength {
    /// 8 px blocks.
    Low,
    /// 10 px blocks (the default).
    #[default]
    Medium,
    /// 14 px blocks.
    High,
    /// A custom block size, clamped to at least [`MIN_BLOCK_PX`].
    Custom(u8),
}

impl Strength {
    /// The block size in reference pixels, never below [`MIN_BLOCK_PX`].
    pub const fn block_px(self) -> u8 {
        match self {
            Self::Low => 8,
            Self::Medium => 10,
            Self::High => 14,
            Self::Custom(px) if px < MIN_BLOCK_PX => MIN_BLOCK_PX,
            Self::Custom(px) => px,
        }
    }
}

/// Parameters of the blur applied to every box.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct BlurParams {
    /// Look of the blur.
    pub style: BlurStyle,
    /// Block size.
    pub strength: Strength,
    /// Padding around each box in reference pixels, `(x, y)`. The soft edge
    /// lives in the padding; the unpadded box is always fully covered.
    pub pad: (u8, u8),
    /// Corner radius in reference pixels.
    pub radius: u8,
    /// Width of the soft edge in reference pixels.
    pub feather: f32,
    /// Whether to add deterministic noise inside the box.
    pub noise: bool,
}

impl Default for BlurParams {
    fn default() -> Self {
        Self {
            style: BlurStyle::PixelBlur,
            strength: Strength::Medium,
            pad: (4, 3),
            radius: 6,
            feather: 1.5,
            noise: true,
        }
    }
}

impl BlurParams {
    /// Whether the parameters are usable: the feather must be a finite,
    /// non-negative number.
    pub fn is_valid(&self) -> bool {
        self.feather.is_finite() && self.feather >= 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::{BlurParams, MIN_BLOCK_PX, Strength};

    #[test]
    fn strengths_map_to_block_sizes() {
        assert_eq!(Strength::Low.block_px(), 8);
        assert_eq!(Strength::Medium.block_px(), 10);
        assert_eq!(Strength::High.block_px(), 14);
        assert_eq!(Strength::Custom(3).block_px(), MIN_BLOCK_PX);
        assert_eq!(Strength::Custom(20).block_px(), 20);
    }

    #[test]
    fn default_matches_the_project_format() {
        let json = serde_json::to_value(BlurParams::default()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "style": "PixelBlur", "strength": "Medium", "pad": [4, 3],
                                "radius": 6, "feather": 1.5, "noise": true })
        );
    }

    #[test]
    fn rejects_negative_feather() {
        let params = BlurParams {
            feather: -1.0,
            ..BlurParams::default()
        };
        assert!(!params.is_valid());
        assert!(BlurParams::default().is_valid());
    }
}
