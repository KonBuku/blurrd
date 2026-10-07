//! Where the HUD sits in a video frame.
//!
//! Detector constants are defined in a 1920 × 1080 reference space;
//! [`HudGeometry`] maps that space onto the source frame. The mapping itself
//! (native 4:3, pillarbox, stretched layouts) is computed by the detector's
//! calibration.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::region::Region;

/// How the game's HUD is laid out in the recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum HudLayout {
    /// Detect the layout from the frames.
    #[default]
    Auto,
    /// 16:9 game resolution in a 16:9 recording.
    Wide16x9,
    /// Native 4:3 recording (for example 1280 × 960).
    Native4x3,
    /// 4:3 game resolution stretched to fill a 16:9 recording.
    Stretched4x3,
    /// 16:10 recording (for example 1920 × 1200).
    Wide16x10,
    /// 4:3 image with black bars of `bar_px` source pixels on both sides.
    Pillarbox {
        /// Width of each black bar in source pixels.
        bar_px: u16,
    },
    /// HUD scaled by the game's HUD scale setting.
    Custom {
        /// HUD scale in percent.
        scale_pct: u16,
    },
}

/// Calibration confidence per region, from 0.0 to 1.0.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(transparent)]
pub struct RegionCalibration(pub BTreeMap<Region, f32>);

/// Mapping of the 1920 × 1080 reference HUD onto the source frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct HudGeometry {
    /// Horizontal scale from reference to source pixels.
    pub sx: f32,
    /// Vertical scale from reference to source pixels.
    pub sy: f32,
    /// Source x of the HUD's left edge (left-anchored elements).
    pub anchor_left: f32,
    /// Source x of the HUD's right edge (right-anchored elements).
    pub anchor_right: f32,
    /// Source x of the HUD's centre (centred elements).
    pub anchor_center: f32,
    /// Vertical offset in source pixels.
    pub oy: f32,
    /// The detected or chosen layout.
    pub layout: HudLayout,
    /// Calibration confidence per region.
    pub per_region: RegionCalibration,
    /// Overall calibration confidence, from 0.0 to 1.0.
    pub confidence: f32,
}

impl Default for HudGeometry {
    /// The identity mapping for a 1920 × 1080 recording, not yet calibrated
    /// (confidence 0).
    fn default() -> Self {
        Self {
            sx: 1.0,
            sy: 1.0,
            anchor_left: 0.0,
            anchor_right: 1920.0,
            anchor_center: 960.0,
            oy: 0.0,
            layout: HudLayout::Auto,
            per_region: RegionCalibration::default(),
            confidence: 0.0,
        }
    }
}
