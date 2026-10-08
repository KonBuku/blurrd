//! HUD and screen detectors of blurrd.
//!
//! The detectors find player names in RGB crops of the HUD regions (kill
//! feed, banner, voice chat, text chat) and of the screen surfaces. They
//! are pure functions of the pixels plus a sequential temporal model, so
//! this crate builds and tests without FFmpeg, ONNX Runtime or a GPU; the
//! engine feeds it crops converted by `blurrd-video`.
//!
//! Detectors:
//!
//! - [`KillFeedDetector`] ([`killfeed`]): killer and victim names in the
//!   kill-feed rows of one frame.
//! - [`BannerDetector`] and [`BannerTracker`] ([`banner`]): the player
//!   name under an ACE or clutch banner, per frame and over the banner's
//!   episode (settle model).
//! - [`VoiceDetector`] ([`voice`]): speaker names in the voice-chat entries
//!   of one frame.
//! - [`ChatDetector`] ([`chat`]): sender names in the text-chat lines of
//!   one frame.
//!
//! Temporal models ([`temporal`]):
//!
//! - [`KillFeedTracker`] ([`temporal::killfeed`]): follows the kill-feed
//!   names through row animations, flashes and fades.
//!
//! The building blocks every detector shares:
//!
//! - [`RgbView`], [`GrayImage`] and [`BitMask`]: the images detectors read
//!   and produce.
//! - [`PixelClasses`]: the per-pixel classes of the port spec (bar colours,
//!   white, pure white, text strokes, opaque bars) with OpenCV-exact HSV
//!   ([`hsv`]).
//! - [`morph`]: erosion, dilation and white top-hat with rectangular
//!   kernels and OpenCV border semantics.
//! - [`components`](mod@components): connected components, runs and
//!   summed-area tables.
//! - [`track`]: template matching with normalised cross-correlation.
//!
//! Every constant is given in reference pixels of a 1920 × 1080 frame, as
//! in the port spec; crops of other sizes are mapped to that space first.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_possible_wrap,
    reason = "crop sizes are at most a few thousand pixels, so coordinates never wrap in i32"
)]

pub mod banner;
pub mod chat;
pub mod classes;
pub mod components;
mod detection;
mod image;
pub mod killfeed;
pub mod morph;
pub mod scene_cut;
pub mod temporal;
pub mod track;
pub mod voice;

pub use banner::{BannerDetector, BannerTracker};
pub use chat::ChatDetector;
pub use classes::{Class, ClassParams, PixelClasses, hsv};
pub use detection::{DetectError, Detection};
pub use image::{BitMask, GrayImage, ImageError, RgbView};
pub use killfeed::KillFeedDetector;
pub use temporal::killfeed::KillFeedTracker;
pub use voice::{VoiceDetector, VoiceTracker};
