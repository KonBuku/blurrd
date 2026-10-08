//! Synthetic HUD fixtures, ground truth and quality metrics for blurrd's
//! tests. Development only: no shipped crate depends on it.
//!
//! Real recordings never enter the repository (architecture plan 11.7), so
//! continuous integration tests the detectors on generated frames instead.
//! The generator draws HUD-like elements with made-up names over procedural
//! backgrounds and knows exactly where every glyph is:
//!
//! - [`Scene`] combines a [`Layout`] (16:9 at any resolution, native 4:3,
//!   pillarbox, 16:10, stretched 4:3), a [`Background`], a [`Palette`]
//!   (default or colour-blind stand-ins) and HUD elements from [`hud`]:
//!   kill-feed rows (up to eight stacked, wipe-in, stack jump, fade),
//!   banners (slide-in and settle), voice-chat speakers (four slots) and
//!   text-chat lines (English, German and Spanish prefixes, wrap, push-up,
//!   fade, system lines). [`Scene::render`] returns a [`Fixture`]: the RGB
//!   frame and its [`FrameTruth`].
//! - Names come from [`synth_name`]: every length up to sixteen
//!   characters, two words, digits, Vietnamese diacritics and, with the
//!   fetched Noto fonts, Hangul, Japanese, Thai, Arabic and Devanagari.
//! - Ground-truth glyph masks are dilated by
//!   [`truth::GLYPH_HALO_REF_PX`] reference pixels (scaled to the layout)
//!   to include the anti-aliasing halo and the drop shadow.
//! - [`metrics`] measures geometric leaks, the high-frequency residual,
//!   over-blur, portrait intrusion, false positives, jitter and flicker
//!   (plan 11.3), with exactly the mask the blur applies.
//! - [`proposals`] merges detector, OCR, slot and pixel-change boxes into
//!   the dense proposals that goldens are reviewed from (plan 11.2).
//! - [`media`] reads the committed media manifest (real recordings by id,
//!   SHA-256, excerpt ranges, case tags and the held-out split) and checks
//!   the recordings in `BLURRD_TEST_MEDIA`; [`golden`] reads reviewed
//!   goldens of real excerpts, turns them into the same [`FrameTruth`] and
//!   scores boxes against them with the same metrics.
//!
//! All geometry follows the measurements in the port spec (reference
//! pixels of a 1920 × 1080 frame). The fonts are OFL-licensed stand-ins;
//! no game font or art is used.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_possible_wrap,
    reason = "frame sizes are at most a few thousand pixels, so pixel coordinates never wrap in i32"
)]
#![allow(
    clippy::many_single_char_names,
    reason = "pixel code uses the conventional names x, y, w, h, r, g, b"
)]

pub mod background;
pub mod font;
pub mod golden;
pub mod hud;
pub mod image;
pub mod layout;
pub mod mask;
pub mod media;
pub mod metrics;
pub mod names;
mod paint;
pub mod palette;
pub mod proposals;
mod rng;
pub mod scene;
pub mod truth;

pub use background::Background;
pub use font::{FontSet, Script};
pub use image::{I420Frame, RgbFrame};
pub use layout::{Anchor, Layout};
pub use mask::Mask;
pub use names::{NameStyle, synth_name};
pub use palette::Palette;
pub use rng::Rng;
pub use scene::{Fixture, Flash, Scene};
pub use truth::{Content, ContentKind, FrameTruth, ItemId, TruthItem};
