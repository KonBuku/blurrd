//! The HUD elements and screen surfaces of the synthetic frames.
//!
//! The gameplay HUD: [`killfeed`], [`banner`], [`voice`] and [`chat`].
//! The screen surfaces (decision D8): [`scoreboard`], [`recap`],
//! [`spectating`] and [`match_end`], all timed by a
//! [`screen::ScreenTiming`]. [`world_tag`] draws the name tags above
//! players' heads that must never be blurred.
//!
//! Each element is described by a plain specification (names, timing,
//! options) and rendered for one [`crate::Layout`] by a [`crate::Scene`].
//! All geometry is in 1920 × 1080 reference pixels, taken from the
//! measurements in the port spec (HUD) or on real recordings (screens);
//! the layout maps it onto the frame.

pub mod banner;
pub mod chat;
pub mod killfeed;
pub mod match_end;
pub mod recap;
pub mod scoreboard;
pub mod screen;
pub mod spectating;
pub mod voice;
pub mod world_tag;
