//! The HUD elements of the synthetic frames.
//!
//! Each element is described by a plain specification (names, timing,
//! options) and rendered for one [`crate::Layout`] by a [`crate::Scene`].
//! All geometry is in 1920 × 1080 reference pixels, taken from the
//! measurements in the port spec; the layout maps it onto the frame.

pub mod banner;
pub mod chat;
pub mod killfeed;
pub mod voice;
