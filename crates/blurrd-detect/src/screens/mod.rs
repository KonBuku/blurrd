//! Screen surfaces: names outside the gameplay HUD (decision D8).
//!
//! These surfaces show player names on full-screen or large overlays:
//!
//! - the scoreboard ([`scoreboard`]),
//! - the death recap and combat report ([`recap`]),
//! - the spectated player's "switch player" widget ([`spectating`]).
//!
//! Each is found from structure only, in three steps:
//!
//! 1. **Gate** ([`gate`]): on the quarter-scale frame, the straight
//!    edges of the surface's panel make a frame a candidate, at a cost
//!    small enough for every frame of normal gameplay.
//! 2. **Read**: only for candidate frames, the surface's native name
//!    zone ([`zone`]) is resampled to reference pixels; the panel's
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

mod detector;
pub mod edges;
pub mod gate;
mod ink;
pub mod recap;
mod sample;
pub mod scoreboard;
pub mod spectating;
pub mod temporal;

use blurrd_core::{RectI, Region};

pub use detector::{ScreenDetector, ScreenSignals};
pub use gate::{GATE_STEP, GATE_THRESHOLD, GateSignal, GateSpec, Gates};
pub use ink::{INK_CONTRAST, INK_MIN_CELLS, INK_MIN_COLUMNS};
pub use sample::{Anchor, FrameView, RefPlane, ScreenMap};
pub use temporal::{Slot, SurfaceFrame, SurfaceTrack};

/// The gate of a screen surface, or `None` for a HUD region.
pub const fn gate_spec(region: Region) -> Option<&'static GateSpec> {
    match region {
        Region::Scoreboard => Some(&scoreboard::GATE),
        Region::DeathRecap => Some(&recap::GATE),
        Region::Spectating => Some(&spectating::GATE),
        _ => None,
    }
}

/// How a screen surface is anchored, or `None` for a HUD region. Boxes of
/// the surface (reference pixels) map to source pixels with
/// [`ScreenMap::to_source`] and this anchor.
pub const fn anchor(region: Region) -> Option<Anchor> {
    match region {
        Region::Scoreboard => Some(Anchor::Center),
        Region::DeathRecap => Some(Anchor::Right),
        Region::Spectating => Some(Anchor::Left),
        _ => None,
    }
}

/// The native zone a surface's reader needs, in source pixels of a frame
/// mapped by `map`, or `None` for a HUD region. The engine crops it
/// (only for frames where the surface's gate makes a candidate) and
/// passes it as a [`FrameView::crop`].
pub fn zone(region: Region, map: &ScreenMap) -> Option<RectI> {
    let rect = match region {
        Region::Scoreboard => scoreboard::ZONE,
        Region::DeathRecap => recap::ZONE,
        Region::Spectating => spectating::ZONE,
        _ => return None,
    };
    Some(map.to_source(rect, anchor(region)?))
}
