//! Screen surfaces: names outside the gameplay HUD (decision D8).
//!
//! These surfaces show player names on full-screen or large overlays;
//! each gets its reader in its own module.
//!
//! Each is found from structure only, in three steps:
//!
//! 1. **Gate** ([`gate`]): on the quarter-scale frame, the straight
//!    edges of the surface's panel make a frame a candidate, at a cost
//!    small enough for every frame of normal gameplay.
//! 2. **Read**: only for candidate frames, the surface's native name
//!    zone is resampled to reference pixels; the panel's
//!    structure is confirmed and placed precisely, and every name slot is
//!    listed with whether it shows ink ([`Slot`]).
//! 3. **Temporal** ([`temporal`]): confirmed frames form runs (short gaps
//!    bridged); during a run, with a pre-roll and a post-roll, every slot
//!    that showed ink gets its slot box (Safe geometry: screen surfaces
//!    have no Tight mode, plan 7.3) from the first fade-in frame to the
//!    end of the fade-out.
//!
//! All constants are reference pixels of a 1920 × 1080 frame, measured
//! on real recordings where the surface appears in them; the calibrated
//! [`HudGeometry`](blurrd_core::HudGeometry) maps them onto any
//! resolution and layout ([`ScreenMap`]). Colours are never used: panel
//! edges are colour steps of any hue, and ink is luminance contrast of
//! any sign, so team colours, colour-blind modes and GPU colour filters
//! do not change what is found. World-space name tags above players'
//! heads have no panel and are never boxed (out of scope, D8).

pub mod edges;
pub mod gate;
mod sample;
pub mod temporal;

pub use gate::{GATE_STEP, GATE_THRESHOLD, GateSignal, GateSpec, Gates};
pub use sample::{Anchor, FrameView, RefPlane, ScreenMap};
pub use temporal::{Slot, SurfaceFrame, SurfaceTrack};
