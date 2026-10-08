//! What the per-frame detectors report.

use blurrd_core::{NameKind, RectI, Region};

/// One player name found in one frame.
///
/// Coordinates are reference pixels of a 1920 × 1080 frame (port spec
/// section 0), half-open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Detection {
    /// The region the name was found in.
    pub region: Region,
    /// What the name is inside its region.
    pub kind: NameKind,
    /// Tight bounds of the name's text strokes.
    pub text: RectI,
    /// The box to blur in this frame: the text plus the padding the region
    /// needs (a kill-feed name gets the full text band of its row, so
    /// descenders and accents are always inside).
    pub rect: RectI,
    /// Top of the kill-feed row the name belongs to, if the region has
    /// rows.
    pub row_top: Option<i32>,
}

/// Why a frame could not be analysed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum DetectError {
    /// The crop does not have the size the detector expects.
    #[error("crop is {actual_width} x {actual_height} px, the detector expects {width} x {height}")]
    CropSize {
        /// Expected width.
        width: u32,
        /// Expected height.
        height: u32,
        /// Width of the given crop.
        actual_width: u32,
        /// Height of the given crop.
        actual_height: u32,
    },
}
