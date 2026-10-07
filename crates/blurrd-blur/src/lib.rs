//! Soft-edged, irreversible box blur on YUV 4:2:0 frames.
//!
//! `blurrd-blur` covers the boxes of a [`blurrd_core::BlurPlan`] directly on
//! the decoded planes (I420, NV12 or P010); frames are never converted to
//! RGB. The steps are:
//!
//! 1. [`PixelParams::new`] resolves the project's [`blurrd_core::BlurParams`]
//!    for one video (block size, padding, feather and radius in source
//!    pixels, scaled by the HUD scale `sy`).
//! 2. [`prepare_boxes`] pads each [`blurrd_core::BlurBox`], aligns it outward
//!    to even coordinates and clamps it to the frame.
//! 3. A [`BlurBackend`] (today [`CpuBlur`]) writes the blur into the frame.
//!
//! Guarantees, covered by tests:
//!
//! - **Coverage:** every pixel of a box's unpadded rectangle gets mask 1.0,
//!   in luma and in the max-pooled chroma; the soft edge lives in the padding.
//! - **Locality:** a box's output depends only on source pixels inside its
//!   padded box. Blurring a region-of-interest crop with its `origin` gives
//!   bit-identical pixels to blurring the full frame, which lets the verify
//!   step run on crops during analysis.
//! - **Determinism:** the same input gives the same output on every run and
//!   thread. The block grid and the optional noise follow the box's
//!   `grid_origin` (the tracked text), so a static name always blurs the
//!   same way and averaging frames recovers nothing.
//! - **Strength floor:** blocks are never smaller than
//!   [`blurrd_core::MIN_BLOCK_PX`] reference pixels.

#![forbid(unsafe_code)]

mod cpu;
mod frame;
pub mod metrics;
mod params;
mod prepare;

use blurrd_core::RectI;

pub use cpu::CpuBlur;
pub use frame::{Plane, PlaneMut, SampleFormat, YuvFrameMut};
pub use params::{
    GAUSSIAN_SIGMA_PER_BLOCK, MAX_SCALE, MIN_SCALE, NOISE_AMPLITUDE, PIXEL_BLUR_SIGMA_PER_BLOCK,
    ParamsError, PixelParams, SampleRange, YuvColour, YuvMatrix,
};
pub use prepare::{PreparedBox, mask_value, prepare_boxes};

/// Why a blur could not be applied. The frame is unchanged when an error is
/// returned.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum BlurError {
    /// The planes of the frame do not describe a valid 4:2:0 frame.
    #[error("invalid frame: {0}")]
    InvalidFrame(String),
    /// The view's origin is odd, so its chroma samples would not line up
    /// with the full frame's.
    #[error("view origin ({x}, {y}) must have even coordinates")]
    OddOrigin {
        /// Horizontal origin.
        x: i32,
        /// Vertical origin.
        y: i32,
    },
    /// A padded box is not completely inside the frame view, so it cannot
    /// be blurred exactly as on the full frame.
    #[error("padded box {padded} is not inside the view {view}")]
    BoxOutsideView {
        /// The padded box.
        padded: RectI,
        /// The part of the full frame the view shows.
        view: RectI,
    },
}

/// Something that blurs prepared boxes into a frame.
pub trait BlurBackend: Send + Sync {
    /// Blurs `boxes` into `frame`.
    ///
    /// `frame` is either the full frame (`origin = (0, 0)`) or a crop of it
    /// whose top-left pixel is at `origin` in full-frame coordinates; box
    /// coordinates are always full-frame coordinates, as produced by
    /// [`prepare_boxes`] for the full frame size. Pixels outside the padded
    /// boxes are never written.
    ///
    /// # Errors
    ///
    /// [`BlurError::InvalidFrame`] for inconsistent planes,
    /// [`BlurError::OddOrigin`] for an odd origin and
    /// [`BlurError::BoxOutsideView`] if a padded box is not inside the view.
    /// The frame is left unchanged on error.
    fn blur(
        &self,
        frame: &mut YuvFrameMut<'_>,
        origin: (i32, i32),
        boxes: &[PreparedBox],
        params: &PixelParams,
    ) -> Result<(), BlurError>;
}
