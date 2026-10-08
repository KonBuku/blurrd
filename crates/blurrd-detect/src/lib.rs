//! HUD and screen detectors of blurrd.
//!
//! The detectors find player names in RGB crops of the HUD regions (kill
//! feed, banner, voice chat, text chat) and of the screen surfaces. They
//! are pure functions of the pixels plus a sequential temporal model, so
//! this crate builds and tests without FFmpeg, ONNX Runtime or a GPU; the
//! engine feeds it crops converted by `blurrd-video`.
//!
//! This version provides the building blocks every detector shares:
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
//!
//! Every constant is given in reference pixels of a 1920 × 1080 frame, as
//! in the port spec; crops of other sizes are mapped to that space first.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_possible_wrap,
    reason = "crop sizes are at most a few thousand pixels, so coordinates never wrap in i32"
)]

pub mod classes;
pub mod components;
mod image;
pub mod morph;

pub use classes::{Class, ClassParams, PixelClasses, hsv};
pub use image::{BitMask, GrayImage, ImageError, RgbView};
